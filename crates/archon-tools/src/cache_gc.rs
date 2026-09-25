//! Bounding the unleased build-cache store.
//!
//! The leased pool is bounded by construction — a fixed number of slots, reused
//! forever. The unleased store is not: it holds one directory per distinct
//! checkout path ever seen, named by a hash of that path, and nothing ever
//! removed one. Every branch worktree that came and went left a multi-gigabyte
//! directory behind, and the store grew without limit.
//!
//! Two mechanisms bound it, in this order.
//!
//! **Garbage collection.** An entry is keyed by the path it caches for, so when
//! that path is gone the entry is provably dead — no heuristic, no age guess.
//! The hash is one-way, so the entry has to say what it belongs to: a marker
//! file written at every acquire records the path and the times. The sweep
//! reads markers, not hashes, and removes only entries whose recorded path the
//! filesystem reports as absent.
//!
//! **A size cap.** Garbage collection cannot catch a long-lived checkout whose
//! cache grows without limit, so a total-size ceiling evicts least-recently-used
//! entries past the bound. Eviction costs a cold build; it never costs work.
//!
//! Both obey one rule: *when in doubt, keep*. Leaking an entry wastes disk.
//! Deleting one that is in use destroys a running build. Every unknown — a
//! missing marker, an unreadable marker, a path whose metadata cannot be read
//! for any reason but "not found" — resolves to keep.
//!
//! Exclusion against concurrent users is the advisory-lock scheme
//! `worktree_ownership` already uses, for the same reasons: cross-process,
//! immune to pid reuse, and self-releasing when a process dies. A user of an
//! entry holds a *shared* lock on a sibling lock file for as long as it builds;
//! the sweep must take the *exclusive* lock before it may remove anything, so
//! an entry with any live user is skipped by construction.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[path = "cache_gc_entry.rs"]
mod cache_gc_entry;

pub use cache_gc_entry::{
    CacheEntryGuard, CacheGcPolicy, EntryMarker, configure, entry_name, open_entry, policy,
    read_marker,
};
use cache_gc_entry::{STAMP_FILE, TRASH_SUFFIX, held, is_entry_name, lock_path};

/// THE LIVENESS RULE. An entry may be collected only when this returns true.
///
/// An entry is dead when, and only when, its marker names a repository path and
/// the filesystem reports that path as `NotFound`. The path recorded is the
/// working directory an agent builds in, so a running agent's entry always
/// resolves to alive — no process can run a command in a directory that does
/// not exist.
///
/// Every other outcome is "alive": no marker, an unreadable or unparseable
/// marker, an empty path, or metadata that cannot be read for any reason other
/// than absence (a permission error, an I/O fault). Those are unknowns, and an
/// unknown must never authorise a deletion.
///
/// An unmounted volume is the one absence that is not evidence. A checkout on
/// `/Volumes/<name>` reads as `NotFound` while its disk is detached, and comes
/// back intact when it is reattached. So `NotFound` counts only once the
/// volume the path lives on is itself present and mounted; see
/// `volume_is_mounted`.
///
/// In particular an entry left by an older build — one with no marker at all —
/// is never collected. Nothing can prove what it belongs to, and nothing can
/// prove it is not in use by a process predating this locking protocol.
fn entry_is_provably_dead(entry: &Path) -> bool {
    let Some(marker) = read_marker(entry) else {
        return false;
    };
    let repository = Path::new(&marker.repository);
    match std::fs::symlink_metadata(repository) {
        Ok(_) => false,
        Err(error) => {
            error.kind() == std::io::ErrorKind::NotFound
                && volume_root(repository).is_none_or(|root| volume_is_mounted(&root))
        }
    }
}

/// `/Volumes/<name>` for a path on a removable or external volume.
fn volume_root(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut components = path.components();
    match (components.next(), components.next(), components.next()) {
        (Some(Component::RootDir), Some(Component::Normal(v)), Some(Component::Normal(name)))
            if v == "Volumes" =>
        {
            Some(Path::new("/Volumes").join(name))
        }
        _ => None,
    }
}

/// True when `root` exists and is a mount point rather than a leftover
/// directory on its parent's filesystem.
///
/// macOS can leave an empty `/Volumes/<name>` behind after an unclean detach,
/// so existence alone would still read an absent disk as present.
fn volume_is_mounted(root: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(root) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let Some(parent) = root.parent() else {
            return true;
        };
        match std::fs::metadata(parent) {
            Ok(parent) => parent.dev() != meta.dev(),
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        meta.is_dir()
    }
}

/// What one sweep did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SweepReport {
    /// Entries removed because their repository is gone.
    pub collected: Vec<String>,
    /// Entries removed to bring the store under the size cap.
    pub evicted: Vec<String>,
    /// Entries kept because someone was using them. Named rather than counted,
    /// and deduplicated: an entry is offered to both passes, and one busy
    /// entry is one fact, not two.
    pub in_use: Vec<String>,
    /// Entries kept because they carry no marker, so nothing can prove them
    /// dead or idle.
    pub unmanaged: usize,
    /// Bytes held by entries with markers, after the sweep.
    pub managed_bytes: u64,
}

impl SweepReport {
    fn note_in_use(&mut self, name: &str) {
        if !self.in_use.iter().any(|held| held == name) {
            self.in_use.push(name.to_string());
        }
    }
}

struct Candidate {
    name: String,
    path: PathBuf,
    last_used: String,
    bytes: u64,
}

/// Remove everything the policy allows, and nothing else.
///
/// Dead entries go first: collecting them is exact, and may leave the store
/// under the cap without evicting anything a living checkout still wants.
pub fn sweep(root: &Path, policy: &CacheGcPolicy) -> SweepReport {
    let mut report = SweepReport::default();
    let mut candidates = collect_candidates(root, &mut report);

    if policy.collect_dead_entries {
        candidates.retain(
            |candidate| match remove_if(root, candidate, entry_is_provably_dead) {
                Removal::Removed => {
                    report.collected.push(candidate.name.clone());
                    false
                }
                Removal::InUse => {
                    report.note_in_use(&candidate.name);
                    true
                }
                Removal::Kept => true,
            },
        );
    }

    let mut total: u64 = candidates.iter().map(|c| c.bytes).sum();
    if policy.max_bytes > 0 && total > policy.max_bytes {
        // Least recently used first. An entry acquired moments ago has just
        // refreshed its marker, so it sorts last and is never the one evicted.
        candidates.sort_by(|a, b| a.last_used.cmp(&b.last_used));
        candidates.retain(|candidate| {
            if total <= policy.max_bytes {
                return true;
            }
            match remove_if(root, candidate, |_| true) {
                Removal::Removed => {
                    total = total.saturating_sub(candidate.bytes);
                    report.evicted.push(candidate.name.clone());
                    false
                }
                Removal::InUse => {
                    report.note_in_use(&candidate.name);
                    true
                }
                Removal::Kept => true,
            }
        });
    }
    report.managed_bytes = total;
    report
}

/// Entries eligible for removal, counting the ones that are not.
///
/// Anything that is not a directory this module named is ignored outright —
/// including the stamp file and every lock file.
fn collect_candidates(root: &Path, report: &mut SweepReport) -> Vec<Candidate> {
    let Ok(dir) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_dir() {
            continue;
        }
        if name.ends_with(TRASH_SUFFIX) {
            // Committed for removal by a sweep that did not finish.
            let _ = std::fs::remove_dir_all(entry.path());
            continue;
        }
        if !is_entry_name(&name) {
            continue;
        }
        let path = entry.path();
        let Some(marker) = read_marker(&path) else {
            report.unmanaged += 1;
            continue;
        };
        let last_used = if marker.last_used_at.is_empty() {
            marker.created_at
        } else {
            marker.last_used_at
        };
        let bytes = directory_bytes(&path);
        candidates.push(Candidate {
            name,
            path,
            last_used,
            bytes,
        });
    }
    candidates
}

enum Removal {
    Removed,
    InUse,
    Kept,
}

/// Remove an entry, but only while holding its exclusive lock and only if
/// `should_remove` still agrees once that lock is held.
///
/// The re-check under the lock is what makes this safe against a user that
/// arrived after the candidate list was built: such a user wrote its marker
/// before starting work, so the re-read sees a live path.
///
/// The directory is renamed before it is removed. The rename is atomic, so a
/// process killed mid-removal leaves a `.trash` directory the next sweep
/// finishes, rather than a half-emptied entry whose marker is already gone and
/// which nothing could ever prove dead again.
fn remove_if(root: &Path, candidate: &Candidate, should_remove: impl Fn(&Path) -> bool) -> Removal {
    let lock = lock_path(root, &candidate.name);
    if held().contains_key(&lock) {
        return Removal::InUse;
    }
    let Ok(file) = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)
    else {
        // Unopenable is not evidence of absence; treat it as in use.
        return Removal::InUse;
    };
    let mut rw = fd_lock::RwLock::new(file);
    let Ok(guard) = rw.try_write() else {
        return Removal::InUse;
    };
    if !should_remove(&candidate.path) {
        drop(guard);
        return Removal::Kept;
    }
    let trash = root.join(format!(
        "{}.{}{TRASH_SUFFIX}",
        candidate.name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&trash);
    if std::fs::rename(&candidate.path, &trash).is_err() {
        drop(guard);
        return Removal::Kept;
    }
    // The entry's name is free once the rename lands, so the lock is released
    // before the slow part. A user arriving now creates a fresh directory; it
    // can never reach the one being deleted, which only this sweep can name.
    drop(guard);
    let _ = std::fs::remove_dir_all(&trash);
    Removal::Removed
}

/// Bytes held below `path`, not following symlinks.
fn directory_bytes(path: &Path) -> u64 {
    let Ok(dir) = std::fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0;
    for entry in dir.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            total += directory_bytes(&entry.path());
        } else if meta.is_file() {
            total += meta.len();
        }
    }
    total
}

/// Sweep `root` if the policy allows it and no sweep has run recently.
///
/// The interval guard is one `stat`, and the sweep itself runs on its own
/// thread: it walks directory metadata for every entry, which is far too slow
/// to sit in front of a command someone is waiting on. Nothing depends on the
/// sweep finishing, and an interrupted one is safe by the `.trash` rename.
pub fn maybe_sweep(root: &Path) {
    let policy = policy();
    if !policy.collect_dead_entries && policy.max_bytes == 0 {
        return;
    }
    if !claim_sweep(root, policy.interval) {
        return;
    }
    let root = root.to_path_buf();
    std::thread::spawn(move || {
        let report = sweep(&root, &policy);
        if !report.collected.is_empty() || !report.evicted.is_empty() {
            tracing::info!(
                collected = ?report.collected,
                evicted = ?report.evicted,
                in_use = ?report.in_use,
                unmanaged = report.unmanaged,
                managed_bytes = report.managed_bytes,
                "build cache: swept unleased store"
            );
        }
    });
}

/// Take the right to sweep, if the last sweep is older than `interval`.
fn claim_sweep(root: &Path, interval: Duration) -> bool {
    let stamp = root.join(STAMP_FILE);
    if let Ok(meta) = std::fs::metadata(&stamp)
        && let Ok(modified) = meta.modified()
        && let Ok(elapsed) = SystemTime::now().duration_since(modified)
        && elapsed < interval
    {
        return false;
    }
    std::fs::write(&stamp, chrono::Utc::now().to_rfc3339()).is_ok()
}

#[cfg(test)]
#[path = "cache_gc_tests.rs"]
mod tests;
