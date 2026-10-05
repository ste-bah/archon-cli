//! A task set's publish lock file (`<pin>.publish.lock`) and the process's
//! record of which thread holds it, and how (Issues 294, 336).
//!
//! Writers — the host's journaled publishes, its recovery, this crate's
//! check-source repins — hold it exclusive. Readers of the frozen chain hold
//! it shared, so reads run side by side and only a writer waits for them. Two
//! opens of the file lock each other even inside one process, so a thread
//! that already holds it and asks again would wait on itself forever: a read
//! nested in any holder on the same thread runs under that holder, a write
//! nested in an exclusive holder runs under it, and a write nested in a read
//! is refused (it cannot upgrade without waiting on its own shared lock).
//!
//! A reader that finds an interrupted publish's journal never settles it under
//! its shared lock: it lets the shared lock go, takes the exclusive one,
//! settles, lets that go, and starts again, so no thread ever holds the shared
//! lock while it waits for the exclusive one.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::ThreadId;

/// How a publish lock is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    /// A reader's: any number at once, never with a writer.
    Shared,
    /// A writer's: alone.
    Exclusive,
}

/// Every publish lock this process holds: its canonical path, the thread that
/// took it and how.
static HELD: Mutex<Vec<(PathBuf, ThreadId, LockMode)>> = Mutex::new(Vec::new());

fn held() -> MutexGuard<'static, Vec<(PathBuf, ThreadId, LockMode)>> {
    HELD.lock().unwrap_or_else(PoisonError::into_inner)
}

fn key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The publish lock file beside the acceptance pin at `pin_path`.
pub fn lock_path(pin_path: &Path) -> PathBuf {
    pin_path.with_extension("publish.lock")
}

/// The journal of a publish of the set pinned at `pin_path`, and the temp a
/// journal state is written to before its rename. The host's `JournalPaths`
/// names the same two files.
pub fn journal_paths(pin_path: &Path) -> [PathBuf; 2] {
    let journal = pin_path.with_extension("publish-journal");
    let mut temp = journal.as_os_str().to_owned();
    temp.push(".tmp");
    [journal, PathBuf::from(temp)]
}

/// Whether a publish of the set pinned at `pin_path` left its journal: one a
/// crash or a failed cleanup interrupted, or one a live writer holds.
pub fn interrupted_publish_left(pin_path: &Path) -> bool {
    journal_paths(pin_path).iter().any(|path| path.exists())
}

/// Whether this thread holds the publish lock at `path`, in either mode.
pub fn held_here(path: &Path) -> bool {
    held_mode_here(path).is_some()
}

/// How this thread holds the publish lock at `path`, if it does.
pub fn held_mode_here(path: &Path) -> Option<LockMode> {
    if !path.exists() {
        return None;
    }
    let (key, here) = (key(path), std::thread::current().id());
    held()
        .iter()
        .find(|(held, owner, _)| *held == key && *owner == here)
        .map(|(_, _, mode)| *mode)
}

/// The host's settlement of a publish a crash interrupted, run while the
/// caller holds the set's publish lock exclusive: the pin and the task root.
pub type Settle = fn(&Path, &Path) -> Result<(), String>;

static SETTLE: OnceLock<Settle> = OnceLock::new();

/// Install the host's settlement for this crate's writers. The first one
/// installed stays; without one, a writer that finds a journal is refused.
pub fn register_settle(settle: Settle) {
    let _ = SETTLE.set(settle);
}

/// How many times a reader settles and starts again before it gives up: a
/// set whose publishes keep being interrupted under it is not readable.
const READ_ATTEMPTS: usize = 3;

/// A held publish lock. Dropping it unlocks the file at once, even while a
/// forked child still shares it (Issue 330), then forgets the holder.
pub struct PublishLockFile {
    file: std::fs::File,
    key: PathBuf,
    owner: ThreadId,
    mode: LockMode,
}

impl PublishLockFile {
    /// Block until the lock at `path` is free of every other holder, then
    /// hold it exclusive: a holder keeps it only for one publish transaction
    /// or one read. The parent directory must exist.
    pub fn acquire(path: &Path) -> Result<Self, String> {
        Self::acquire_as(path, LockMode::Exclusive)
    }

    /// Block until no writer holds the lock at `path`, then hold it shared
    /// beside any other reader. The parent directory must exist.
    pub fn acquire_shared(path: &Path) -> Result<Self, String> {
        Self::acquire_as(path, LockMode::Shared)
    }

    fn acquire_as(path: &Path, mode: LockMode) -> Result<Self, String> {
        if held_here(path) {
            return Err(format!(
                "this thread already holds the publish lock {}; a nested acquisition would wait on itself",
                path.display()
            ));
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .map_err(|error| format!("opening publish lock {}: {error}", path.display()))?;
        match mode {
            LockMode::Shared => file.lock_shared(),
            LockMode::Exclusive => file.lock(),
        }
        .map_err(|error| format!("locking publish lock {}: {error}", path.display()))?;
        let lock = Self {
            file,
            key: key(path),
            owner: std::thread::current().id(),
            mode,
        };
        held().push((lock.key.clone(), lock.owner, mode));
        Ok(lock)
    }

    /// How this lock is held.
    pub fn mode(&self) -> LockMode {
        self.mode
    }

    /// Hold the publish lock of the set pinned at `pin_path` exclusive for one
    /// repin, a publish a crash interrupted settled first (by the host's
    /// [`register_settle`]d settlement, for the set at `tasks_root`). `None`
    /// when this thread already holds it exclusive (the repin runs under its
    /// holder, which settled) or when no set of this project was ever frozen
    /// (the pin's directory does not exist, so nothing is published). Refused
    /// when this thread holds it shared: a write inside a read.
    pub fn hold(pin_path: &Path, tasks_root: Option<&Path>) -> Result<Option<Self>, String> {
        let path = lock_path(pin_path);
        match held_mode_here(&path) {
            Some(LockMode::Exclusive) => return Ok(None),
            Some(LockMode::Shared) => {
                return Err(format!(
                    "this thread reads the task set under the shared publish lock {}; a write inside that read is refused, as it would wait on the read",
                    path.display()
                ));
            }
            None if !path.parent().is_some_and(Path::is_dir) => return Ok(None),
            None => {}
        }
        let lock = Self::acquire(&path)?;
        match (SETTLE.get(), tasks_root) {
            (Some(settle), Some(tasks_root)) => settle(pin_path, tasks_root)?,
            _ if interrupted_publish_left(pin_path) => {
                return Err(format!(
                    "a publish of this task set was interrupted and left {}; it is settled before anything else writes the set — run any `archon workflow` command on the set (launch, resume, or a freeze) to settle it, then retry",
                    journal_paths(pin_path)[0].display()
                ));
            }
            _ => {}
        }
        Ok(Some(lock))
    }

    /// Hold the publish lock of the set pinned at `pin_path` shared for one
    /// read. `None` when this thread already holds it (the read runs under
    /// its holder) or when no set of this project was ever frozen.
    pub fn hold_shared(pin_path: &Path) -> Result<Option<Self>, String> {
        let path = lock_path(pin_path);
        if held_here(&path) || !path.parent().is_some_and(Path::is_dir) {
            return Ok(None);
        }
        Self::acquire_shared(&path).map(Some)
    }

    /// Hold the publish lock of the set pinned at `pin_path` shared for one
    /// read of a set no interrupted publish is left in. A journal found under
    /// the shared lock is settled by `settle` under the exclusive lock, taken
    /// only after the shared one is let go; the state is then read again
    /// under a fresh shared lock. `settle` runs at most [`READ_ATTEMPTS`]
    /// times; its error is the read's. The caller makes sure this thread
    /// holds no publish lock of the set.
    pub fn acquire_shared_settled<E>(
        pin_path: &Path,
        mut settle: impl FnMut() -> Result<(), E>,
        error: impl Fn(String) -> E,
    ) -> Result<Self, E> {
        let path = lock_path(pin_path);
        for _ in 0..READ_ATTEMPTS {
            let shared = Self::acquire_shared(&path).map_err(&error)?;
            if !interrupted_publish_left(pin_path) {
                return Ok(shared);
            }
            drop(shared);
            let exclusive = Self::acquire(&path).map_err(&error)?;
            // A writer may have finished between the two: check again.
            if interrupted_publish_left(pin_path) {
                settle()?;
            }
            drop(exclusive);
        }
        Err(error(format!(
            "a publish of the task set left {} again after each of {READ_ATTEMPTS} settlements; it is not read until its publishes stop being interrupted",
            journal_paths(pin_path)[0].display()
        )))
    }
}

impl Drop for PublishLockFile {
    fn drop(&mut self) {
        let mut held = held();
        if let Some(index) = held
            .iter()
            .position(|(key, owner, _)| *key == self.key && *owner == self.owner)
        {
            held.swap_remove(index);
        }
        drop(held);
        if let Err(error) = self.file.unlock() {
            tracing::warn!(%error, "publish lock could not be unlocked; it is released when closed");
        }
    }
}

#[cfg(test)]
#[path = "task_set_publish_lock_tests.rs"]
mod tests;
