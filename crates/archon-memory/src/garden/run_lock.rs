//! The single-run lock: at most one consolidation pass over one store.
//!
//! Consolidation prunes, decays and merges a user's stored memories. Two passes
//! running over one store at the same time is the failure that loses data:
//! both read the same candidate set, both decide the same memory is the loser of
//! a merge or below the staleness floor, and both act. Nothing downstream
//! detects it, because a memory that is gone leaves no evidence that two writers
//! rather than one removed it.
//!
//! Until now nothing prevented that. Consolidation ran from session start and
//! from `/garden`, so two Archon launches seconds apart, or a launch racing a
//! typed `/garden`, already overlapped. Adding an unattended scheduler on top of
//! that would have made the overlap routine rather than occasional, which is why
//! this exists before the scheduler does.
//!
//! # Why a file lock rather than a row in the store
//!
//! The obvious alternative is a lease row in the memory graph: create it, hold
//! it, delete it. Every lease design then needs an expiry, because a process
//! that dies holding one never deletes it — and an expiry is a guess about how
//! long a legitimate run takes. Guess short and a slow-but-healthy run has its
//! lock stolen, which is precisely the concurrent-writer state the lock exists
//! to prevent. Guess long and a crash blocks consolidation for hours.
//!
//! An OS advisory lock has no expiry to guess, because the kernel releases it
//! when the holding process exits — crash, kill, or panic alike. A run that
//! fails, times out or is killed therefore leaves *no* lock behind and the next
//! run proceeds, which is the required property stated exactly.
//!
//! It also writes nothing to the memory graph. The 13 memories destroyed by an
//! earlier version of this subsystem were destroyed because one phase wrote
//! `RelatedTo` edges to mean "undecided" and another phase read them as "merge
//! these". A lock kept in a file outside the graph cannot be misread by a phase,
//! because no phase can see it.
//!
//! # Two layers, because one is not enough
//!
//! [`with_run_lock`] takes both:
//!
//! 1. A process-local registry of held paths. Two tasks in *this* process are
//!    refused here, immediately and without touching the filesystem.
//! 2. An exclusive `flock`/`LockFileEx` on a lock file. A second *process* is
//!    refused here.
//!
//! Layer 1 is not redundant. `flock` is held per open file description and
//! `LockFileEx` per handle, so two opens in one process do contend on both
//! platforms today — but that is a property of two libc implementations, not of
//! the invariant, and the invariant is load-bearing enough to state locally
//! rather than inherit.
//!
//! # Scoped, not returned
//!
//! The lock is handed to a closure rather than returned as a guard. A returned
//! guard can be forgotten, `mem::forget`-ed, or stored somewhere that outlives
//! the run; a closure cannot outlive its own call. The release path runs from
//! `Drop`, so it also runs when the closure panics.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use tracing::{debug, warn};

use crate::types::MemoryError;

/// Lock file name, alongside `memory.port` and `memory.lock` in the data dir.
///
/// The data dir is what makes two processes agree they are looking at one store:
/// it already holds the port file they use to find each other. Keying the lock
/// on the database path instead would leave two processes that reached the same
/// store through different path spellings unsynchronised.
pub const GARDEN_RUN_LOCK_FILE: &str = "garden-run.lock";

/// Paths this process currently holds. See the module docs, layer 1.
fn held_paths() -> &'static Mutex<HashSet<PathBuf>> {
    static HELD: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Lock a poisoned-tolerant mutex.
///
/// A panic while the registry mutex is held would otherwise poison it and
/// wedge every future consolidation permanently. The set is a plain collection
/// of paths with no invariant a panic can violate half-way, so recovering the
/// inner value is sound and strictly better than never consolidating again.
fn lock_registry() -> std::sync::MutexGuard<'static, HashSet<PathBuf>> {
    held_paths()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What [`with_run_lock`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunLockOutcome<T> {
    /// The lock was taken and the closure ran. Carries whatever it returned.
    Ran(T),
    /// Another consolidation holds the lock. Nothing ran, and nothing was
    /// written.
    Busy,
}

impl<T> RunLockOutcome<T> {
    /// The closure's result, or `None` if the lock was held elsewhere.
    pub fn ran(self) -> Option<T> {
        match self {
            Self::Ran(value) => Some(value),
            Self::Busy => None,
        }
    }

    /// Whether the run was declined because another pass holds the lock.
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Busy)
    }
}

/// Removes this process's claim on a path when it goes out of scope.
///
/// A `Drop` impl rather than a call at the end of [`with_run_lock`], so a panic
/// inside the consolidation closure still releases. Without it, one panicking
/// pass would leave the process unable to consolidate for its whole lifetime,
/// and the failure would present as "consolidation silently stopped happening".
struct ProcessClaim(PathBuf);

impl Drop for ProcessClaim {
    fn drop(&mut self) {
        lock_registry().remove(&self.0);
    }
}

/// Run `consolidation` holding the single-run lock for this store, or report
/// [`RunLockOutcome::Busy`] if another pass already holds it.
///
/// `lock_path` is a file that will be created if absent; only its lock state is
/// used, never its contents. Both layers described in the module docs are taken
/// before the closure runs and released after it returns *or panics*.
///
/// This DECLINES rather than waits. A blocked consolidation is not worth
/// queueing: the work the other pass is doing is the same work this one would
/// do, over the same store, and by the time the lock frees the candidate set has
/// been consumed. Waiting would also mean a background job could stall on a
/// `/garden` a user is watching, or vice versa.
///
/// # Errors
///
/// Only for a lock file that cannot be created or opened — a missing directory
/// or a permissions problem. A lock held elsewhere is `Ok(Busy)`, not an error:
/// it is the expected outcome under contention, and callers must not treat it
/// as a fault.
pub fn with_run_lock<T>(
    lock_path: &Path,
    consolidation: impl FnOnce() -> T,
) -> Result<RunLockOutcome<T>, MemoryError> {
    if let Some(parent) = lock_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    // Created before canonicalising, because `canonicalize` fails on a path that
    // does not exist yet and the first run is always the one that does not.
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;

    // Canonical, so two spellings of one path — a relative path, a symlinked
    // data dir, `C:\x` versus `c:\x` — are one key in the registry rather than
    // two, and therefore contend as the single store they actually are.
    let key = std::fs::canonicalize(lock_path).unwrap_or_else(|_| lock_path.to_path_buf());

    // Layer 1: this process.
    if !lock_registry().insert(key.clone()) {
        debug!(
            path = %lock_path.display(),
            "garden: consolidation already running in this process; declining"
        );
        return Ok(RunLockOutcome::Busy);
    }
    let _claim = ProcessClaim(key);

    // Layer 2: every other process. Held for exactly as long as `guard` lives,
    // and released by the OS if this process dies before that.
    let mut file_lock = fd_lock::RwLock::new(file);
    let guard = match file_lock.try_write() {
        Ok(guard) => guard,
        Err(error) => {
            debug!(
                path = %lock_path.display(),
                %error,
                "garden: consolidation lock held by another process; declining"
            );
            return Ok(RunLockOutcome::Busy);
        }
    };

    let result = consolidation();
    drop(guard);
    Ok(RunLockOutcome::Ran(result))
}

/// The lock file for a store whose coordination files live in `data_dir`.
pub fn run_lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join(GARDEN_RUN_LOCK_FILE)
}

/// Report a declined run once, at a level that makes a permanently-stuck lock
/// visible without making ordinary contention noisy.
///
/// Contention is expected — two launches seconds apart, a scheduler tick during
/// a typed `/garden` — and a `warn!` on every occurrence trains people to
/// ignore it. What is not expected is *every* run being declined, which is what
/// a leaked lock looks like, so the message names the file to check.
pub fn log_declined(lock_path: &Path) {
    debug!(
        path = %lock_path.display(),
        "garden: another consolidation holds the run lock; this pass did nothing"
    );
}

/// Warn that a consolidation could not even attempt to take the lock.
///
/// Separated from [`log_declined`] because the two must not read alike: declined
/// means the lock worked, this means it did not run at all.
pub fn log_unavailable(lock_path: &Path, error: &MemoryError) {
    warn!(
        path = %lock_path.display(),
        %error,
        "garden: consolidation run lock unavailable; skipping this pass rather than \
         running unprotected"
    );
}

#[cfg(test)]
#[path = "run_lock_tests.rs"]
mod run_lock_tests;
