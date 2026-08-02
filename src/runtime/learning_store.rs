//! Process-lifetime cache for runtime governed-learning stores.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use anyhow::Result;
use cozo::DbInstance;

#[derive(Default)]
struct Cache {
    entries: HashMap<std::path::PathBuf, Entry>,
}

enum Entry {
    Opening,
    Ready(Arc<DbInstance>),
}

struct StoreCache {
    cache: Mutex<Cache>,
    changed: Condvar,
}

static STORE_CACHE: OnceLock<StoreCache> = OnceLock::new();

fn store_cache() -> &'static StoreCache {
    STORE_CACHE.get_or_init(|| StoreCache {
        cache: Mutex::new(Cache::default()),
        changed: Condvar::new(),
    })
}

pub(crate) fn acquire_default() -> Result<Arc<DbInstance>> {
    acquire_for_path(&crate::command::store_paths::learning_db_path())
}

#[cfg(test)]
pub(crate) fn acquire_for_dir(working_dir: &Path) -> Result<Arc<DbInstance>> {
    acquire_for_path(&crate::command::store_paths::learning_db_path_for_dir(
        working_dir,
    ))
}

pub(crate) async fn acquire_default_async() -> Result<Arc<DbInstance>> {
    acquire_for_path_async(crate::command::store_paths::learning_db_path()).await
}

pub(crate) async fn acquire_for_dir_async(working_dir: &Path) -> Result<Arc<DbInstance>> {
    acquire_for_path_async(crate::command::store_paths::learning_db_path_for_dir(
        working_dir,
    ))
    .await
}

async fn acquire_for_path_async(path: std::path::PathBuf) -> Result<Arc<DbInstance>> {
    tokio::task::spawn_blocking(move || acquire_for_path(&path))
        .await
        .map_err(|error| anyhow::anyhow!("learning store acquisition task failed: {error}"))?
}

#[cfg(test)]
pub(crate) async fn acquire_for_path_with_async(
    path: &Path,
    open: impl Fn(&Path) -> Result<Arc<DbInstance>> + Send + Sync + 'static,
) -> Result<Arc<DbInstance>> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || acquire_for_path_with_arc(&path, open))
        .await
        .map_err(|error| anyhow::anyhow!("learning store acquisition task failed: {error}"))?
}

pub(crate) fn acquire_for_path(path: &Path) -> Result<Arc<DbInstance>> {
    acquire_for_path_with_arc(path, open_and_ensure)
}

fn open_and_ensure(path: &Path) -> Result<Arc<DbInstance>> {
    let path_text = path.to_string_lossy();
    let config = archon_cozo::CozoGuardConfig::for_interactive_db_path(path);
    let db =
        archon_cozo::open_sqlite_guarded_instance(&path_text, "open learning db", config)?.db_arc();
    archon_learning::schema::ensure_learning_schema(&db)?;
    Ok(db)
}

#[cfg(test)]
pub(crate) fn acquire_for_path_with(
    path: &Path,
    open: impl Fn(&Path) -> Result<Arc<DbInstance>> + Send + Sync + 'static,
) -> Result<Arc<DbInstance>> {
    acquire_for_path_with_arc(path, open)
}

pub(crate) fn acquire_for_path_with_arc(
    path: &Path,
    open: impl Fn(&Path) -> Result<Arc<DbInstance>> + Send + Sync + 'static,
) -> Result<Arc<DbInstance>> {
    let key = archon_cozo::canonical_resource_path(path)?;
    let cache = store_cache();
    let mut cache_guard = lock_recovering_poison(&cache.cache);

    loop {
        match cache_guard.entries.get(&key) {
            Some(Entry::Ready(db)) => return Ok(Arc::clone(db)),
            Some(Entry::Opening) => {
                cache_guard = wait_recovering_poison(&cache.changed, cache_guard);
            }
            None => {
                cache_guard.entries.insert(key.clone(), Entry::Opening);
                break;
            }
        }
    }
    drop(cache_guard);

    let opened = catch_unwind(AssertUnwindSafe(|| open(&key)));
    let mut cache_guard = lock_recovering_poison(&cache.cache);
    match opened {
        Ok(Ok(db)) => {
            cache_guard
                .entries
                .insert(key, Entry::Ready(Arc::clone(&db)));
            cache.changed.notify_all();
            Ok(db)
        }
        Ok(Err(error)) => {
            cache_guard.entries.remove(&key);
            cache.changed.notify_all();
            Err(error)
        }
        Err(panic) => {
            cache_guard.entries.remove(&key);
            cache.changed.notify_all();
            drop(cache_guard);
            resume_unwind(panic)
        }
    }
}

fn lock_recovering_poison<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn wait_recovering_poison<'a, T>(
    condvar: &Condvar,
    guard: std::sync::MutexGuard<'a, T>,
) -> std::sync::MutexGuard<'a, T> {
    match condvar.wait(guard) {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
pub(crate) fn clear_for_tests(path: &Path) {
    let key = archon_cozo::canonical_resource_path(path).expect("normalize test cache key");
    let cache = store_cache();
    let mut cache_guard = lock_recovering_poison(&cache.cache);
    assert!(
        !matches!(cache_guard.entries.get(&key), Some(Entry::Opening)),
        "cannot clear a learning store cache entry while it is opening"
    );
    cache_guard.entries.remove(&key);
    cache.changed.notify_all();
}

#[cfg(test)]
#[path = "learning_store_persistence_tests.rs"]
mod persistence_tests;
#[cfg(test)]
#[path = "learning_store_tests.rs"]
mod tests;
