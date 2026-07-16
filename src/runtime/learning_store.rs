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

pub(crate) fn acquire_for_dir(working_dir: &Path) -> Result<Arc<DbInstance>> {
    acquire_for_path(&crate::command::store_paths::learning_db_path_for_dir(
        working_dir,
    ))
}

pub(crate) fn acquire_for_path(path: &Path) -> Result<Arc<DbInstance>> {
    acquire_for_path_with(path, open_and_ensure)
}

fn open_and_ensure(path: &Path) -> Result<DbInstance> {
    let path_text = path.to_string_lossy();
    let db = archon_learning::cozo_guard::open_sqlite_guarded(&path_text, "open learning db")?;
    archon_learning::cozo_guard::ensure_learning_schema_guarded(&db, path)?;
    Ok(db)
}

pub(crate) fn acquire_for_path_with(
    path: &Path,
    open: impl Fn(&Path) -> Result<DbInstance> + Send + Sync + 'static,
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

    let opened = catch_unwind(AssertUnwindSafe(|| open(&key).map(Arc::new)));
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
#[path = "learning_store_tests.rs"]
mod tests;
