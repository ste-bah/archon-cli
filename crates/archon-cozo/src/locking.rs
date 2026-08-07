use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum WriteLockKey {
    Fallback,
    Path(PathBuf),
}

static COZO_PROCESS_WRITE_LOCKS: OnceLock<Mutex<HashMap<WriteLockKey, Arc<Mutex<()>>>>> =
    OnceLock::new();

thread_local! {
    static HELD_WRITE_LOCKS: RefCell<Vec<WriteLockKey>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn write_lock_key(path: Option<&Path>) -> Result<WriteLockKey> {
    match path {
        Some(path) => Ok(WriteLockKey::Path(canonical_resource_path(path)?)),
        None => Ok(WriteLockKey::Fallback),
    }
}

pub(crate) fn process_write_lock(key: &WriteLockKey) -> Arc<Mutex<()>> {
    let locks = COZO_PROCESS_WRITE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = lock_recovering_poison(locks);
    Arc::clone(
        locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(()))),
    )
}

pub(crate) fn write_lock_is_held(key: &WriteLockKey) -> bool {
    HELD_WRITE_LOCKS.with(|held| held.borrow().contains(key))
}

pub(crate) struct HeldWriteLock {
    key: WriteLockKey,
}

impl HeldWriteLock {
    pub(crate) fn enter(key: WriteLockKey) -> Self {
        HELD_WRITE_LOCKS.with(|held| held.borrow_mut().push(key.clone()));
        Self { key }
    }
}

impl Drop for HeldWriteLock {
    fn drop(&mut self) {
        HELD_WRITE_LOCKS.with(|held| {
            let popped = held.borrow_mut().pop();
            debug_assert_eq!(popped.as_ref(), Some(&self.key));
        });
    }
}

/// Return a stable path key for a possibly absent resource.
///
/// Existing resources are canonicalized completely. For absent resources, the
/// nearest existing ancestor is canonicalized so relative paths, `.`/`..`, and
/// symlinked parents resolve to one process-wide resource identity without
/// creating directories.
pub(crate) fn canonical_resource_path(path: impl AsRef<Path>) -> Result<PathBuf> {
    let mut path = absolute_normalized_path(path.as_ref())?;

    for _ in 0..40 {
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let target = std::fs::read_link(&path)?;
                path = if target.is_absolute() {
                    target
                } else {
                    path.parent().unwrap_or_else(|| Path::new(".")).join(target)
                };
                path = absolute_normalized_path(&path)?;
            }
            Ok(_) => return Ok(simplified_path(path.canonicalize()?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return canonicalize_missing_path(&path);
            }
            Err(error) => return Err(error.into()),
        }
    }

    Err(anyhow!(
        "too many symbolic links while resolving {}",
        path.display()
    ))
}

fn canonicalize_missing_path(path: &Path) -> Result<PathBuf> {
    let mut unresolved = Vec::new();
    let mut existing = path;

    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            break;
        };
        unresolved.push(name.to_os_string());
        existing = existing.parent().unwrap_or_else(|| Path::new("."));
    }

    let mut normalized = canonicalize_existing_path(existing)?;
    for component in unresolved.iter().rev() {
        normalized.push(component);
    }
    Ok(simplified_path(normalized))
}

/// Drop the Windows verbatim (`\\?\`) prefix that `canonicalize` adds.
///
/// A verbatim path is handed to the filesystem with no Win32 normalisation at
/// all, which means a forward slash is no longer accepted as a separator. Any
/// consumer that appends one then produces a path the OS rejects. RocksDB does
/// exactly that — it opens `<root>/LOG` — so every vector-store test failed
/// with "The filename, directory name, or volume label syntax is incorrect"
/// against a path like `\\?\C:\...\.tmpRxzgNS/LOG`.
///
/// This is a no-op off Windows, and it deliberately leaves two cases alone:
///   * non-disk verbatim prefixes (`\\?\UNC\...`, device paths), where the
///     prefix is not merely decorative;
///   * paths at or beyond `MAX_PATH`, where the prefix is the only reason the
///     path works at all.
#[cfg(windows)]
fn simplified_path(path: PathBuf) -> PathBuf {
    use std::path::{Component, Prefix};

    const MAX_PATH: usize = 260;

    if path.as_os_str().len() >= MAX_PATH {
        return path;
    }
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path;
    };
    let Prefix::VerbatimDisk(letter) = prefix.kind() else {
        return path;
    };
    let mut simplified = PathBuf::from(format!("{}:\\", letter as char));
    simplified.extend(components.filter(|component| !matches!(component, Component::RootDir)));
    simplified
}

#[cfg(not(windows))]
fn simplified_path(path: PathBuf) -> PathBuf {
    path
}

fn absolute_normalized_path(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(lexically_normalized_path(&path))
}

fn canonicalize_existing_path(path: &Path) -> Result<PathBuf> {
    let mut unresolved = Vec::new();
    let mut existing = path;

    loop {
        match existing.canonicalize() {
            Ok(mut normalized) => {
                for component in unresolved.iter().rev() {
                    normalized.push(component);
                }
                return Ok(normalized);
            }
            Err(error) => {
                let Some(name) = existing.file_name() else {
                    return Err(error.into());
                };
                unresolved.push(name.to_os_string());
                let Some(parent) = existing.parent() else {
                    return Err(error.into());
                };
                existing = parent;
            }
        }
    }
}

fn lexically_normalized_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

pub(crate) fn lock_recovering_poison<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Poll interval floor and ceiling for a blocking write-lock acquire.
///
/// `fd_lock` only offers a non-blocking `try_write` plus an unbounded blocking
/// `write`, and an unbounded wait is exactly what turns a stuck holder into a
/// hung process. So the blocking variant polls instead, backing off from
/// [`ACQUIRE_POLL_FLOOR`] to [`ACQUIRE_POLL_CEILING`] so a short handover costs
/// microseconds while a long queue does not burn a core.
const ACQUIRE_POLL_FLOOR: Duration = Duration::from_millis(1);
const ACQUIRE_POLL_CEILING: Duration = Duration::from_millis(25);

fn open_write_lock_file(path: &Path, context: &str) -> Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|error| {
            anyhow!(
                "{context}: open Cozo write lock {}: {error}",
                path.display()
            )
        })
}

pub(crate) fn with_write_lock<T>(
    path: &Path,
    context: &str,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let file = open_write_lock_file(path, context)?;
    let mut lock = fd_lock::RwLock::new(file);
    let _guard = lock.try_write().map_err(|error| {
        anyhow!(
            "{context}: Cozo write lock unavailable at {}: {error}",
            path.display()
        )
    })?;
    tracing::trace!(context, lock_path = %path.display(), "acquired Cozo write lock");
    run()
}

/// The phrase every bounded acquire failure carries, whichever layer expired.
///
/// A wedged holder is deliberately *not* a retryable busy signal -- we already
/// waited the whole budget, so another 19s of backoff would only delay the
/// diagnosis. But a caller that can degrade (skip one file rather than abandon
/// a walk) still has to tell "lost the store" apart from "the schema is wrong",
/// and it should not do that by re-spelling this sentence at the call site.
/// [`crate::is_store_contention`] matches on this constant instead.
pub(crate) const WRITE_LOCK_WAIT_EXPIRED: &str = "was still held";

/// Run `run` while holding the write lock for `path`, waiting up to `wait` for
/// a current holder to finish.
///
/// This is the serialising sibling of [`with_write_lock`], which fails fast so
/// its callers can retry with backoff. Callers that need an actual mutual
/// exclusion window — a read-then-reserve compare-and-set that must not be
/// interleaved — cannot use fail-fast semantics, because losing the race is not
/// an error they can recover from without redoing the read.
///
/// Three properties matter here:
///
/// * **Bounded.** A holder that never releases (a crashed peer that leaked the
///   OS lock, a wedged writer) surfaces as an error naming the lock file rather
///   than as a hang. `wait` is a ceiling on the acquire, not on `run`.
/// * **Re-entrant.** On Windows `LockFileEx` byte-range locks conflict between
///   handles *within one process*, so a thread that already owns this lock and
///   re-enters would block on itself forever. The thread-local ownership set
///   maintained by [`HeldWriteLock`] — the same one `run_guarded_once` uses to
///   skip re-locking — is consulted first, and a re-entrant call simply runs
///   inline under the lock it already holds.
/// * **Two-layer.** The process-wide mutex keyed on the same canonical path
///   orders threads within this process, so only one of them ever contends for
///   the OS lock. Without it, N waiting threads would poll one file lock N
///   times over, and the intra-process conflict above would make the outcome
///   depend on `LockFileEx` queueing rather than on arrival order.
pub(crate) fn with_write_lock_blocking<T>(
    path: &Path,
    context: &str,
    wait: Duration,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let key = write_lock_key(Some(path))?;
    if write_lock_is_held(&key) {
        tracing::trace!(
            context,
            lock_path = %path.display(),
            "reusing Cozo write lock already held by this thread"
        );
        return run();
    }

    let deadline = Instant::now() + wait;
    let process_lock = process_write_lock(&key);
    let _process_guard = acquire_process_lock(&process_lock, path, context, wait, deadline)?;
    let _held_lock = HeldWriteLock::enter(key);
    let mut lock = fd_lock::RwLock::new(open_write_lock_file(path, context)?);
    let mut run = Some(run);
    let mut backoff = ACQUIRE_POLL_FLOOR;

    loop {
        match lock.try_write() {
            Ok(_guard) => {
                tracing::trace!(
                    context,
                    lock_path = %path.display(),
                    "acquired blocking Cozo write lock"
                );
                let run = run
                    .take()
                    .expect("the blocking write lock body is taken exactly once");
                return run();
            }
            Err(error) => {
                let Some(remaining) = deadline
                    .checked_duration_since(Instant::now())
                    .filter(|remaining| !remaining.is_zero())
                else {
                    return Err(anyhow!(
                        "{context}: Cozo write lock at {} was still held after waiting {}ms: {error}",
                        path.display(),
                        wait.as_millis()
                    ));
                };
                std::thread::sleep(backoff.min(remaining));
                backoff = (backoff * 2).min(ACQUIRE_POLL_CEILING);
            }
        }
    }
}

/// Take the process-wide mutex for a lock key without waiting forever.
///
/// `std::sync::Mutex` has no timed acquire, so this polls `try_lock` under the
/// same deadline as the file lock. A poisoned mutex is recovered rather than
/// propagated, matching [`lock_recovering_poison`]: the data is `()`, so a
/// panicking holder leaves nothing inconsistent behind.
fn acquire_process_lock<'a>(
    lock: &'a Mutex<()>,
    path: &Path,
    context: &str,
    wait: Duration,
    deadline: Instant,
) -> Result<MutexGuard<'a, ()>> {
    let mut backoff = ACQUIRE_POLL_FLOOR;
    loop {
        match lock.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(poisoned)) => return Ok(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => {}
        }
        let Some(remaining) = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
        else {
            return Err(anyhow!(
                "{context}: Cozo write lock at {} was still held by this process after waiting {}ms",
                path.display(),
                wait.as_millis()
            ));
        };
        std::thread::sleep(backoff.min(remaining));
        backoff = (backoff * 2).min(ACQUIRE_POLL_CEILING);
    }
}
