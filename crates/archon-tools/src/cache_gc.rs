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
//! **Marker-less entries.** An entry with no usable marker — left by a build
//! that predates markers, or by one killed between creating the directory and
//! writing its marker — can never be proved dead, and once excluded from both
//! passes it was never removed at all: the store grew past its cap with such
//! entries. Such an entry is removed when no one holds its lock *and* nothing
//! anywhere inside it changed within `unmarked_idle`. It counts toward the
//! cap, and the cap may evict it only under the same two conditions.
//!
//! All of it obeys one rule: *when in doubt, keep*. Leaking an entry wastes
//! disk. Deleting one that is in use destroys a running build. A held lock, a
//! recent write, a subtree that cannot be read, a path whose metadata cannot be
//! read for any reason but "not found" — each resolves to keep.
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

#[path = "cache_gc_scan.rs"]
mod cache_gc_scan;

pub use cache_gc_entry::{
    CacheEntryGuard, CacheGcPolicy, EntryMarker, configure, entry_name, open_entry, policy,
    read_marker,
};
use cache_gc_entry::{STAMP_FILE, TRASH_SUFFIX, held, lock_path};
use cache_gc_scan::{Candidate, Kind, collect_candidates, idle_verdict, unmarked_removable};

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
/// unknown must never authorise a deletion by this rule.
///
/// An unmounted volume is the one absence that is not evidence. A checkout on
/// `/Volumes/<name>` reads as `NotFound` while its disk is detached, and comes
/// back intact when it is reattached. So `NotFound` counts only once the
/// volume the path lives on is itself present and mounted; see
/// `volume_is_mounted`.
///
/// An entry with no usable marker is never collected by this rule. It is
/// handled by the idle rule instead (`cache_gc_scan::idle_verdict`): nothing
/// can prove what it belongs to, but its lock and its mtimes can prove nobody
/// is using it.
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
    /// Marked entries removed because their repository is gone.
    pub collected: Vec<String>,
    /// Marker-less entries removed because no one held their lock and nothing
    /// in them changed within the idle threshold.
    pub reclaimed: Vec<String>,
    /// Entries removed to bring the store under the size cap.
    pub evicted: Vec<String>,
    /// Entries kept because someone was using them. Named rather than counted,
    /// and deduplicated: an entry is offered to both passes, and one busy
    /// entry is one fact, not two.
    pub in_use: Vec<String>,
    /// Marker-less entries still present after the sweep.
    pub unmanaged: usize,
    /// Entries whose marker exists but is unreadable or malformed, with why.
    /// They are treated as marker-less.
    pub invalid_markers: Vec<(String, String)>,
    /// Bytes held by every entry, marked or not, after the sweep.
    pub total_bytes: u64,
}

impl SweepReport {
    fn note_in_use(&mut self, name: &str) {
        if !self.in_use.iter().any(|held| held == name) {
            self.in_use.push(name.to_string());
        }
    }
}

/// Remove everything the policy allows, and nothing else.
///
/// Dead and idle marker-less entries go first: removing them is exact, and may
/// leave the store under the cap without evicting anything a living checkout
/// still wants. The cap then counts every entry, marked or not.
///
/// Every removal is logged with its reason, and every entry left standing is
/// logged once with the reason it was kept.
pub fn sweep(root: &Path, policy: &CacheGcPolicy) -> SweepReport {
    let mut report = SweepReport::default();
    let mut candidates = collect_candidates(root, &mut report);
    let idle = policy.unmarked_idle;

    if policy.collect_dead_entries {
        candidates.retain_mut(|c| {
            let removed = match &c.kind {
                Kind::Marked => try_remove(root, c, &mut report, "checkout is gone", |p| {
                    if entry_is_provably_dead(p) {
                        Ok(())
                    } else {
                        Err("checkout present, or its absence is not proof".into())
                    }
                }),
                Kind::Unmarked(_) => try_remove_unmarked(root, c, &mut report, idle, "idle"),
            };
            if removed {
                match c.kind {
                    Kind::Marked => report.collected.push(c.name.clone()),
                    Kind::Unmarked(_) => report.reclaimed.push(c.name.clone()),
                }
            }
            !removed
        });
    }

    let mut total: u64 = candidates.iter().map(|c| c.scan.bytes).sum();
    if policy.max_bytes > 0 && total > policy.max_bytes {
        // Least recently used first. An entry acquired moments ago has just
        // refreshed its marker, so it sorts last and is never the one evicted.
        candidates.sort_by_key(|c| c.last_used);
        candidates.retain_mut(|c| {
            if total <= policy.max_bytes {
                return true;
            }
            let removed = match &c.kind {
                Kind::Marked => try_remove(root, c, &mut report, "over the size cap", |_| Ok(())),
                Kind::Unmarked(_) => try_remove_unmarked(root, c, &mut report, idle, "over cap"),
            };
            if removed {
                total = total.saturating_sub(c.scan.bytes);
                report.evicted.push(c.name.clone());
            }
            !removed
        });
        if total > policy.max_bytes {
            tracing::warn!(
                total_bytes = total,
                max_bytes = policy.max_bytes,
                "build cache: unleased store is still over its cap; every remaining entry is in use, fresh, or unprovable"
            );
        }
    }

    for c in &candidates {
        tracing::info!(
            entry = %c.name,
            bytes = c.scan.bytes,
            marker = kind_label(&c.kind),
            reason = %c.kept_because,
            "build cache: kept unleased entry"
        );
    }
    report.unmanaged = candidates
        .iter()
        .filter(|c| matches!(c.kind, Kind::Unmarked(_)))
        .count();
    report.total_bytes = total;
    report
}

fn kind_label(kind: &Kind) -> &str {
    match kind {
        Kind::Marked => "present",
        Kind::Unmarked(why) => why,
    }
}

/// A marker-less entry is removed only when it is provably idle — first by the
/// scan already taken, so a fresh entry is never even locked — and again under
/// its exclusive lock, which also proves no current user holds it.
fn try_remove_unmarked(
    root: &Path,
    c: &mut Candidate,
    report: &mut SweepReport,
    idle: Duration,
    pass: &str,
) -> bool {
    match idle_verdict(&c.scan, idle) {
        Err(why) => {
            c.kept_because = format!("marker-less; {why}");
            false
        }
        Ok(for_how_long) => {
            let reason = format!(
                "marker-less, unlocked, idle {}s >= {}s ({pass})",
                for_how_long.as_secs(),
                idle.as_secs()
            );
            try_remove(root, c, report, &reason, |p| unmarked_removable(p, idle))
        }
    }
}

/// Remove `c` through `remove_if`, logging the removal with `reason`, or
/// recording why it stays. True when removed.
fn try_remove(
    root: &Path,
    c: &mut Candidate,
    report: &mut SweepReport,
    reason: &str,
    should_remove: impl Fn(&Path) -> Result<(), String>,
) -> bool {
    match remove_if(root, c, should_remove) {
        Removal::Removed => {
            tracing::info!(
                entry = %c.name,
                bytes = c.scan.bytes,
                marker = kind_label(&c.kind),
                reason,
                "build cache: removed unleased entry"
            );
            true
        }
        Removal::InUse(why) => {
            report.note_in_use(&c.name);
            c.kept_because = why;
            false
        }
        Removal::Kept(why) => {
            c.kept_because = why;
            false
        }
    }
}

enum Removal {
    Removed,
    InUse(String),
    Kept(String),
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
/// finishes, rather than a half-emptied entry.
fn remove_if(
    root: &Path,
    candidate: &Candidate,
    should_remove: impl Fn(&Path) -> Result<(), String>,
) -> Removal {
    let lock = lock_path(root, &candidate.name);
    if held().contains_key(&lock) {
        return Removal::InUse("in use by this process".into());
    }
    let file = match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)
    {
        Ok(file) => file,
        // Unopenable is not evidence of absence; treat it as in use.
        Err(error) => return Removal::InUse(format!("lock file unopenable: {error}")),
    };
    let mut rw = fd_lock::RwLock::new(file);
    let Ok(guard) = rw.try_write() else {
        return Removal::InUse("lock held by another user".into());
    };
    if let Err(why) = should_remove(&candidate.path) {
        drop(guard);
        return Removal::Kept(why);
    }
    let trash = root.join(format!(
        "{}.{}{TRASH_SUFFIX}",
        candidate.name,
        std::process::id()
    ));
    if let Err(error) = std::fs::remove_dir_all(&trash)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        drop(guard);
        tracing::warn!(trash = %trash.display(), %error, "build cache: stale trash could not be cleared");
        return Removal::Kept(format!("stale trash could not be cleared: {error}"));
    }
    if let Err(error) = std::fs::rename(&candidate.path, &trash) {
        drop(guard);
        tracing::warn!(entry = %candidate.name, %error, "build cache: rename for removal failed");
        return Removal::Kept(format!("rename for removal failed: {error}"));
    }
    // The entry's name is free once the rename lands, so the lock is released
    // before the slow part. A user arriving now creates a fresh directory; it
    // can never reach the one being deleted, which only this sweep can name.
    drop(guard);
    if let Err(error) = std::fs::remove_dir_all(&trash) {
        // Committed: the next sweep finishes it.
        tracing::warn!(trash = %trash.display(), %error, "build cache: removal left trash for the next sweep");
    }
    Removal::Removed
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
        tracing::info!(
            collected = ?report.collected,
            reclaimed = ?report.reclaimed,
            evicted = ?report.evicted,
            in_use = ?report.in_use,
            unmanaged = report.unmanaged,
            invalid_markers = ?report.invalid_markers,
            total_bytes = report.total_bytes,
            "build cache: swept unleased store"
        );
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
