use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

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

pub(crate) fn with_write_lock<T>(
    path: &Path,
    context: &str,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
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
        })?;
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
