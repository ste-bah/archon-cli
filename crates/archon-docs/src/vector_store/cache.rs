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
            register_exit_drain();
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

unsafe extern "C" {
    /// Register a callback to run when the process exits normally.
    ///
    /// Declared directly rather than pulled in through `libc` so this crate does
    /// not grow a dependency for one symbol that every C runtime we target --
    /// glibc, musl, and the MSVC CRT -- already exports.
    fn atexit(callback: extern "C" fn()) -> std::ffi::c_int;
}

/// Close every cached store before the C++ runtime tears RocksDB down.
///
/// This cache is reachable from a `'static`, so nothing ever drops its entries:
/// the `DB` handles are still open when the process exits. RocksDB gives every
/// open database a periodic-task timer thread, and that thread outlives `main`
/// for exactly the same reason. Once the exit sequence destroys RocksDB's own
/// statics the thread is running against wreckage -- its stats dump calls
/// `GetPropertyInfo`, which searches the destroyed `InternalStats::ppt_name_to_info`
/// map, gets back a null `DBPropertyInfo*`, and dereferences it. The release
/// build has `assert(property_info != nullptr)` compiled out, so the first read
/// off that null pointer lands at address 0x8 and the process dies of SIGSEGV
/// *after* the last line of user code has run. Under `cargo nextest`, which
/// reports the signal a test process died from, that surfaces as a test which
/// prints `ok` and is then recorded as SIGSEGV.
///
/// Draining here closes each database, which cancels the periodic task and joins
/// the timer thread while the process is still whole.
extern "C" fn close_cached_stores_at_exit() {
    drop(drain_entries(cache()));
}

/// Take every entry out of `cache`, handing the caller the last strong reference
/// each one held.
///
/// The entries come back rather than being dropped in place so that the closing
/// happens with the lock released: closing a database blocks until its
/// background work drains, and nothing needs to be excluded from the map for
/// that.
fn drain_entries(cache: &StoreCache) -> Vec<(PathBuf, Entry)> {
    cache
        .entries
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .drain()
        .collect()
}

/// Put [`close_cached_stores_at_exit`] ahead of every RocksDB static destructor.
///
/// Exit callbacks run last-registered-first, so ordering is decided by when this
/// runs relative to RocksDB's own registrations. Some of those are made during
/// dynamic initialisation, before `main`; others come from function-local
/// statics that RocksDB touches for the first time part-way through a
/// `DB::open`. Registering *after* an open has succeeded therefore clears both
/// groups.
///
/// Re-registering on every successful open is deliberate, not an oversight. A
/// later open can be the one that first touches some RocksDB static, and only a
/// registration made after *that* open is ordered ahead of it. The duplicate
/// callbacks cost one uncontended lock each, because the first one to run leaves
/// the map empty.
fn register_exit_drain() {
    // SAFETY: `atexit` only requires a pointer to an `extern "C"` function that
    // takes no arguments and returns nothing, which is what is passed here. The
    // callback touches nothing but this module's own `'static` cache.
    let _ = unsafe { atexit(close_cached_stores_at_exit) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exit drain has to leave the cache holding no strong reference at all,
    /// because that reference is the only reason the RocksDB handle is still
    /// open when the process starts tearing itself down.
    #[test]
    fn draining_releases_the_last_cached_reference() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(DocVectorStore::open(temp.path()).unwrap());
        let cache = StoreCache {
            entries: Mutex::new(HashMap::new()),
            changed: Condvar::new(),
        };
        cache
            .entries
            .lock()
            .unwrap()
            .insert(temp.path().to_path_buf(), Entry::Ready(Arc::clone(&store)));

        let drained = drain_entries(&cache);

        assert_eq!(drained.len(), 1);
        assert!(cache.entries.lock().unwrap().is_empty());
        assert_eq!(Arc::strong_count(&store), 2);
        drop(drained);
        assert_eq!(Arc::strong_count(&store), 1);
    }
}
