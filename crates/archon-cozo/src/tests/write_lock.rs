//! Acquiring, queueing behind, and releasing the file write lock.
//!
//! Covers both acquire modes — fail-fast and blocking-with-timeout — plus the
//! two properties the callers depend on: re-entrancy, so a guarded operation
//! can take the lock it already holds, and exact release, so the thread-local
//! ownership set unwinds and a later acquire does not silently run unlocked.
//!
//! [`stale_guarded_database_registry_entries_are_pruned`] belongs here because
//! the registry is what resolves a database to the lock these tests take.
use std::fs::OpenOptions;
use std::sync::Arc;
use std::time::Duration;

use cozo::ScriptMutability;

use super::*;

#[test]
fn stale_guarded_database_registry_entries_are_pruned() {
    let database = GuardedDbInstance::new(
        DbInstance::new("mem", "", "").unwrap(),
        CozoGuardConfig::default(),
    );
    let key = Arc::as_ptr(&database.db) as usize;
    let weak = Arc::downgrade(&database.db);
    assert!(guarded_config_for(database.db()).is_some());

    drop(database);
    assert_eq!(weak.strong_count(), 0);

    assert!(guard_registry::registered_database_keys().contains(&key));
    let replacement = DbInstance::new("mem", "", "").unwrap();
    assert!(guarded_config_for(&replacement).is_none());
    assert!(!guard_registry::registered_database_keys().contains(&key));
}

#[test]
fn write_lock_rejects_second_writer() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("test.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .unwrap();
    let mut lock = fd_lock::RwLock::new(file);
    let _guard = lock.try_write().unwrap();

    let error = with_write_lock(&path, "test lock", || Ok(())).unwrap_err();

    assert!(is_retryable_cozo_error(&format!("{error:#}")));
}

#[test]
fn blocking_write_lock_waits_for_the_current_holder() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("blocking.lock");
    let acquired = Arc::new(std::sync::Barrier::new(2));
    let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let holder_acquired = Arc::clone(&acquired);
    let holder_released = Arc::clone(&released);
    let holder_path = path.clone();

    let holder = std::thread::spawn(move || {
        with_write_lock(&holder_path, "holder", || {
            holder_acquired.wait();
            std::thread::sleep(Duration::from_millis(200));
            // Set inside the closure so the flag is observable only while the
            // lock is still held; a non-blocking acquire would see `false`.
            holder_released.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    });

    acquired.wait();
    with_write_lock_blocking(&path, "waiter", || {
        assert!(
            released.load(std::sync::atomic::Ordering::SeqCst),
            "blocking acquire returned while the lock was still held"
        );
        Ok(())
    })
    .unwrap();
    holder.join().unwrap();
}

/// The shape the routed crates now produce: a guarded mutable operation that
/// holds the write lock across a whole multi-statement transaction rather than
/// a single `:put`. A blocking waiter must queue behind it and then succeed,
/// not trip its bounded wait.
#[test]
fn blocking_write_lock_queues_behind_a_long_guarded_critical_section() {
    let temp = tempfile::tempdir().unwrap();
    let lock_path = temp.path().join("critical-section.lock");
    let config = CozoGuardConfig::default().with_write_lock_path(&lock_path);
    let entered = Arc::new(std::sync::Barrier::new(2));
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let holder_entered = Arc::clone(&entered);
    let holder_finished = Arc::clone(&finished);

    let holder = std::thread::spawn(move || {
        run_guarded(
            "long transaction",
            ScriptMutability::Mutable,
            &config,
            || {
                holder_entered.wait();
                std::thread::sleep(Duration::from_millis(250));
                holder_finished.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap();
    });

    entered.wait();
    with_write_lock_blocking_timeout(&lock_path, "waiter", Duration::from_secs(10), || {
        assert!(
            finished.load(std::sync::atomic::Ordering::SeqCst),
            "waiter entered the critical section while the guarded operation held it"
        );
        Ok(())
    })
    .unwrap();
    holder.join().unwrap();
}

#[test]
fn blocking_write_lock_reports_a_stuck_holder_instead_of_hanging() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("stuck.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .unwrap();
    // A raw `fd_lock` hold, deliberately outside this crate's bookkeeping, so
    // the acquire cannot mistake it for a re-entrant call on this thread.
    let mut lock = fd_lock::RwLock::new(file);
    let _guard = lock.try_write().unwrap();

    let started = std::time::Instant::now();
    let error = with_write_lock_blocking_timeout(&path, "stuck", Duration::from_millis(50), || {
        Ok::<(), anyhow::Error>(())
    })
    .unwrap_err();

    assert!(started.elapsed() >= Duration::from_millis(50));
    let message = format!("{error:#}");
    assert!(
        message.contains("still held after waiting 50ms"),
        "{message}"
    );
    // A wedged holder is not a busy database: retrying it for another 19s of
    // backoff would only delay the diagnosis.
    assert!(!is_retryable_cozo_error(&message), "{message}");
}

#[test]
fn blocking_write_lock_is_reentrant_inside_a_guarded_operation() {
    let temp = tempfile::tempdir().unwrap();
    let lock_path = temp.path().join("guarded.lock");
    let config = CozoGuardConfig::default().with_write_lock_path(&lock_path);

    // The guarded operation holds the OS lock for `lock_path`. On Windows that
    // byte-range lock conflicts with a second handle in this same process, so a
    // naive re-acquire here would block until the timeout.
    let ran = run_guarded("outer", ScriptMutability::Mutable, &config, || {
        with_write_lock_blocking_timeout(&lock_path, "inner", Duration::from_millis(50), || {
            Ok(true)
        })
    })
    .unwrap();

    assert!(ran);
}

#[test]
fn blocking_write_lock_nests_within_itself() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("nested.lock");

    let depth = with_write_lock_blocking(&path, "outer", || {
        with_write_lock_blocking_timeout(&path, "inner", Duration::from_millis(50), || {
            with_write_lock_blocking_timeout(&path, "innermost", Duration::from_millis(50), || {
                Ok(3)
            })
        })
    })
    .unwrap();

    assert_eq!(depth, 3);
}

#[test]
fn blocking_write_lock_is_released_for_the_next_acquirer() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sequential.lock");

    for _ in 0..3 {
        with_write_lock_blocking_timeout(&path, "sequential", Duration::from_millis(50), || Ok(()))
            .unwrap();
    }

    // The thread-local ownership set must unwind exactly, or a later acquire
    // would silently run without the lock.
    with_write_lock(&path, "fail-fast after blocking", || Ok(())).unwrap();
}
