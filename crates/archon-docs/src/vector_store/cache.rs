use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use anyhow::Result;

use super::DocVectorStore;

enum Entry {
    Opening,
    Ready(Arc<DocVectorStore>),
}

struct StoreCache {
    entries: Mutex<HashMap<PathBuf, Entry>>,
    changed: Condvar,
}

fn cache() -> &'static StoreCache {
    static CACHE: OnceLock<StoreCache> = OnceLock::new();
    CACHE.get_or_init(|| StoreCache {
        entries: Mutex::new(HashMap::new()),
        changed: Condvar::new(),
    })
}

pub(super) fn acquire(path: &Path) -> Result<Arc<DocVectorStore>> {
    acquire_with(path, |key| DocVectorStore::open(key).map(Arc::new))
}

pub(super) fn acquire_with(
    path: &Path,
    open: impl FnOnce(&Path) -> Result<Arc<DocVectorStore>>,
) -> Result<Arc<DocVectorStore>> {
    let key = archon_cozo::canonical_resource_path(path)?;
    let cache = cache();
    let mut entries = cache
        .entries
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    loop {
        match entries.get(&key) {
            Some(Entry::Ready(store)) => return Ok(Arc::clone(store)),
            Some(Entry::Opening) => {
                entries = cache
                    .changed
                    .wait(entries)
                    .unwrap_or_else(|error| error.into_inner());
            }
            None => {
                entries.insert(key.clone(), Entry::Opening);
                break;
            }
        }
    }
    drop(entries);

    let opened = catch_unwind(AssertUnwindSafe(|| open(&key)));
    let mut entries = cache
        .entries
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    match opened {
        Ok(Ok(store)) => {
            entries.insert(key, Entry::Ready(Arc::clone(&store)));
            cache.changed.notify_all();
            Ok(store)
        }
        Ok(Err(error)) => {
            entries.remove(&key);
            cache.changed.notify_all();
            Err(error)
        }
        Err(panic) => {
            entries.remove(&key);
            cache.changed.notify_all();
            drop(entries);
            resume_unwind(panic)
        }
    }
}
