//! What a sweep learns about each entry before it decides anything: its size,
//! the newest modification anywhere inside it, and what its marker says.
//!
//! Split from `cache_gc` to hold the 500-line ceiling. Nothing here removes
//! anything; `cache_gc` decides and removes.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::SweepReport;
use super::cache_gc_entry::{MarkerState, TRASH_SUFFIX, is_entry_name, marker_state};

/// One walk of an entry's tree.
#[derive(Debug, Clone, Default)]
pub(super) struct Scan {
    /// Bytes in regular files, not following symlinks.
    pub(super) bytes: u64,
    /// Newest mtime of the entry directory or anything below it.
    pub(super) newest: Option<SystemTime>,
    /// Paths whose metadata or listing could not be read.
    pub(super) unreadable: usize,
    pub(super) first_error: Option<String>,
}

impl Scan {
    fn note_error(&mut self, path: &Path, error: &std::io::Error) {
        self.unreadable += 1;
        if self.first_error.is_none() {
            self.first_error = Some(format!("{}: {error}", path.display()));
        }
    }

    fn note_mtime(&mut self, meta: &std::fs::Metadata) {
        if let Ok(modified) = meta.modified() {
            self.newest = Some(self.newest.map_or(modified, |n| n.max(modified)));
        }
    }
}

/// Walk `entry`, counting bytes and finding the newest mtime. Iterative, so a
/// deep build tree cannot exhaust the stack. Every failure is counted, never
/// skipped silently: an unread subtree could hide a recent write.
pub(super) fn scan_entry(entry: &Path) -> Scan {
    let mut scan = Scan::default();
    match std::fs::symlink_metadata(entry) {
        Ok(meta) => scan.note_mtime(&meta),
        Err(error) => {
            scan.note_error(entry, &error);
            return scan;
        }
    }
    let mut pending = vec![entry.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let listing = match std::fs::read_dir(&dir) {
            Ok(listing) => listing,
            Err(error) => {
                scan.note_error(&dir, &error);
                continue;
            }
        };
        for child in listing {
            let child = match child {
                Ok(child) => child,
                Err(error) => {
                    scan.note_error(&dir, &error);
                    continue;
                }
            };
            // `DirEntry::metadata` does not follow symlinks.
            let meta = match child.metadata() {
                Ok(meta) => meta,
                Err(error) => {
                    scan.note_error(&child.path(), &error);
                    continue;
                }
            };
            scan.note_mtime(&meta);
            if meta.is_dir() {
                pending.push(child.path());
            } else if meta.is_file() {
                scan.bytes += meta.len();
            }
        }
    }
    scan
}

/// Whether an entry with no usable marker is provably idle: every path in it
/// was read, and none changed within `threshold`. `Err` says why not.
pub(super) fn idle_verdict(scan: &Scan, threshold: Duration) -> Result<Duration, String> {
    if threshold.is_zero() {
        return Err("marker-less removal is disabled (idle threshold is 0)".into());
    }
    if scan.unreadable > 0 {
        return Err(format!(
            "{} path(s) could not be read, so idleness is unprovable (first: {})",
            scan.unreadable,
            scan.first_error.as_deref().unwrap_or("unknown")
        ));
    }
    let Some(newest) = scan.newest else {
        return Err("no modification time could be read".into());
    };
    match SystemTime::now().duration_since(newest) {
        Ok(idle) if idle >= threshold => Ok(idle),
        Ok(idle) => Err(format!(
            "fresh: last modified {}s ago, idle threshold {}s",
            idle.as_secs(),
            threshold.as_secs()
        )),
        Err(_) => Err("last modified in the future; clock skew is not idleness".into()),
    }
}

/// Re-checked under the entry's exclusive lock, so a user that adopted the
/// entry (and so wrote a marker) or wrote into it since the scan keeps it.
pub(super) fn unmarked_removable(entry: &Path, threshold: Duration) -> Result<(), String> {
    if let MarkerState::Present(_) = marker_state(entry) {
        return Err("a marker appeared since the scan".into());
    }
    idle_verdict(&scan_entry(entry), threshold).map(|_| ())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Kind {
    /// Governed by the liveness rule and the cap.
    Marked,
    /// No usable marker: governed by lock plus idleness. Carries why.
    Unmarked(String),
}

pub(super) struct Candidate {
    pub(super) name: String,
    pub(super) path: PathBuf,
    pub(super) kind: Kind,
    /// Marker's last use for a marked entry; newest mtime otherwise.
    pub(super) last_used: SystemTime,
    pub(super) scan: Scan,
    /// Why the entry still exists, for the log line once the sweep ends.
    pub(super) kept_because: String,
}

fn parse_stamp(stamp: &str) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc3339(stamp)
        .ok()
        .map(SystemTime::from)
}

/// Every entry in `root`. Anything that is not a directory this module named
/// is ignored outright — including the stamp file and every lock file.
pub(super) fn collect_candidates(root: &Path, report: &mut SweepReport) -> Vec<Candidate> {
    let dir = match std::fs::read_dir(root) {
        Ok(dir) => dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            tracing::warn!(root = %root.display(), %error, "build cache: cannot list unleased store");
            return Vec::new();
        }
    };
    let mut candidates = Vec::new();
    for entry in dir {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::warn!(root = %root.display(), %error, "build cache: unreadable store listing entry");
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(kind) = entry.file_type() else {
            tracing::warn!(entry = %name, "build cache: cannot read file type; skipped");
            continue;
        };
        if !kind.is_dir() {
            continue;
        }
        if name.ends_with(TRASH_SUFFIX) {
            // Committed for removal by a sweep that did not finish.
            if let Err(error) = std::fs::remove_dir_all(entry.path()) {
                tracing::warn!(entry = %name, %error, "build cache: could not finish an interrupted removal");
            }
            continue;
        }
        if !is_entry_name(&name) {
            continue;
        }
        candidates.push(candidate(name, entry.path(), report));
    }
    candidates
}

fn candidate(name: String, path: PathBuf, report: &mut SweepReport) -> Candidate {
    let scan = scan_entry(&path);
    if scan.unreadable > 0 {
        tracing::warn!(
            entry = %name,
            unreadable = scan.unreadable,
            first_error = scan.first_error.as_deref().unwrap_or(""),
            "build cache: part of an unleased entry could not be read; size and age are lower bounds"
        );
    }
    let now = SystemTime::now();
    let newest = scan.newest.unwrap_or(now);
    let (kind, last_used) = match marker_state(&path) {
        MarkerState::Present(marker) => {
            let stamp = if marker.last_used_at.is_empty() {
                marker.created_at
            } else {
                marker.last_used_at
            };
            let last_used = parse_stamp(&stamp).unwrap_or_else(|| {
                tracing::warn!(entry = %name, stamp, "build cache: marker time unparseable; ordering by newest mtime");
                newest
            });
            (Kind::Marked, last_used)
        }
        MarkerState::Missing => (Kind::Unmarked("no marker".into()), newest),
        MarkerState::Invalid(why) => {
            tracing::warn!(entry = %name, error = %why, "build cache: invalid marker; treating the entry as marker-less");
            report.invalid_markers.push((name.clone(), why.clone()));
            (Kind::Unmarked(format!("invalid marker ({why})")), newest)
        }
    };
    Candidate {
        name,
        path,
        kind,
        last_used,
        scan,
        kept_because: "no removal rule applied".into(),
    }
}
