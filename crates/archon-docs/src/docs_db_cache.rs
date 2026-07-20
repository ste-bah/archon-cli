use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use anyhow::{Context, Result};
use cozo::DbInstance;

pub(crate) fn acquire(path: &Path) -> Result<Arc<DbInstance>> {
    acquire_with(path, open)
}

fn open(path: &Path) -> Result<Arc<DbInstance>> {
    crate::configure_cozo_write_lock_for_db(path);
    let display = path.display();
    let db = DbInstance::new("sqlite", path, "")
        .map_err(|error| anyhow::anyhow!("open document store at {display}: {error}"))?;
    crate::schema::ensure_doc_schema(&db)
        .with_context(|| format!("ensure document schema at {display}"))?;
    Ok(Arc::new(db))
}

fn acquire_with(
    path: &Path,
    open: impl FnOnce(&Path) -> Result<Arc<DbInstance>>,
) -> Result<Arc<DbInstance>> {
    let key = archon_cozo::canonical_resource_path(path)?;
    let cache = cache();
    let mut entries = cache
        .entries
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    loop {
        match entries.get(&key) {
            Some(Entry::Ready(db)) => return Ok(Arc::clone(db)),
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
        Ok(Ok(db)) => {
            entries.insert(key, Entry::Ready(Arc::clone(&db)));
            cache.changed.notify_all();
            Ok(db)
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

enum Entry {
    Opening,
    Ready(Arc<DbInstance>),
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn same_canonical_path_reuses_one_database() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("docs.db");

        let first = acquire(&path).unwrap();
        let second = acquire(&temp.path().join(".").join("docs.db")).unwrap();

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[cfg(unix)]
    #[test]
    fn final_file_symlinks_reuse_one_database() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real.db");
        let alias = temp.path().join("alias.db");
        std::fs::File::create(&real).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();

        let first = acquire(&real).unwrap();
        let second = acquire(&alias).unwrap();

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn concurrent_acquisition_waits_for_one_in_flight_open() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("concurrent.db");
        let (entered_tx, entered_rx) = mpsc::channel();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let open_count = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let entered_tx = entered_tx.clone();
                let release = Arc::clone(&release);
                let open_count = Arc::clone(&open_count);
                std::thread::spawn(move || {
                    acquire_with(&path, |key| {
                        open_count.fetch_add(1, Ordering::SeqCst);
                        entered_tx.send(()).unwrap();
                        let (released, changed) = &*release;
                        let mut released = released.lock().unwrap();
                        while !*released {
                            released = changed.wait(released).unwrap();
                        }
                        open(key)
                    })
                    .unwrap()
                })
            })
            .collect();
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            entered_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "a second opener entered while the first remained blocked"
        );
        let (released, changed) = &*release;
        *released.lock().unwrap() = true;
        changed.notify_all();
        let databases: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();

        assert_eq!(open_count.load(Ordering::SeqCst), 1);
        assert!(
            databases
                .iter()
                .all(|database| Arc::ptr_eq(&databases[0], database))
        );
    }

    #[test]
    fn failed_open_can_be_retried() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("retry.db");
        std::fs::create_dir(&path).unwrap();

        assert!(acquire(&path).is_err());
        std::fs::remove_dir(&path).unwrap();

        assert!(acquire(&path).is_ok());
    }

    #[test]
    fn panicking_open_can_be_retried() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("panic.db");

        assert!(
            std::panic::catch_unwind(|| {
                let _ = acquire_with(&path, |_| -> Result<_> {
                    panic!("synthetic document store open panic")
                });
            })
            .is_err()
        );

        assert!(acquire(&path).is_ok());
    }
}
