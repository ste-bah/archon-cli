use std::cell::Cell;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use cozo::ScriptMutability;

use super::*;

#[test]
fn write_lock_path_is_sibling_sidecar() {
    let path = PathBuf::from("/tmp/archon-data.db");
    assert_eq!(
        write_lock_path_for_db(&path),
        PathBuf::from("/tmp/archon-data.db.archon-cozo-write.lock")
    );
}

#[test]
fn deriving_write_lock_path_does_not_create_database_parent() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("missing").join("nested");
    let db_path = parent.join("learning.db");

    let lock_path = write_lock_path_for_db(&db_path);

    assert!(!parent.exists(), "lock-path derivation created directories");
    assert_eq!(lock_path, parent.join("learning.db.archon-cozo-write.lock"));
}

#[test]
fn retryable_errors_include_sqlite_and_file_lock_variants() {
    assert!(is_retryable_cozo_error("database is locked (code 5)"));
    assert!(is_retryable_cozo_error("sqlite_busy"));
    assert!(is_retryable_cozo_error("Cozo write lock unavailable"));
    assert!(!is_retryable_cozo_error("relation not found"));
}

#[test]
fn retryable_errors_match_only_precise_busy_signals() {
    for message in [
        "database is locked",
        "database table is locked",
        "locked (code 5)",
        "code: Some(5)",
        "SQLITE_BUSY",
        "would-block",
        "write-lock unavailable",
        "write lock unavailable",
        "poison error",
    ] {
        assert!(is_retryable_cozo_error(message), "{message}");
    }

    for message in [
        "code 50",
        "code 500",
        "code: Some(50)",
        "code: Some(500)",
        "unrelated code 5 prefix",
    ] {
        assert!(!is_retryable_cozo_error(message), "{message}");
    }
}

#[test]
fn sync_guarded_retry_retries_then_succeeds() {
    let attempts = Cell::new(0);
    let config = CozoGuardConfig {
        max_attempts: 2,
        initial_backoff: Duration::ZERO,
        max_backoff: Duration::ZERO,
        write_lock_path: None,
    };

    let value = run_guarded("sync retry", ScriptMutability::Immutable, &config, || {
        attempts.set(attempts.get() + 1);
        if attempts.get() == 1 {
            return Err(anyhow!("database is locked"));
        }
        Ok("success")
    })
    .unwrap();

    assert_eq!(value, "success");
    assert_eq!(attempts.get(), 2);
}

#[tokio::test(start_paused = true)]
async fn async_guarded_retry_yields_and_succeeds() {
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let attempts_for_run = Arc::clone(&attempts);
    let config = CozoGuardConfig {
        max_attempts: 2,
        initial_backoff: Duration::from_secs(1),
        max_backoff: Duration::from_secs(1),
        write_lock_path: None,
    };
    let yielded = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let yielded_for_task = Arc::clone(&yielded);
    let task = tokio::spawn(async move {
        tokio::task::yield_now().await;
        yielded_for_task.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    let retry = run_guarded_async(
        "async retry",
        ScriptMutability::Immutable,
        &config,
        move || {
            let attempt = attempts_for_run.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if attempt == 0 {
                return Err(anyhow!("database is locked"));
            }
            Ok("success")
        },
    );
    tokio::pin!(retry);

    tokio::select! {
        result = &mut retry => panic!("retry unexpectedly completed: {result:?}"),
        _ = tokio::task::yield_now() => {}
    }
    task.await.unwrap();
    assert!(yielded.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);

    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(retry.await.unwrap(), "success");
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn async_guarded_terminal_error_does_not_retry() {
    let attempts = Cell::new(0);
    let config = CozoGuardConfig {
        max_attempts: 2,
        initial_backoff: Duration::from_secs(1),
        max_backoff: Duration::from_secs(1),
        write_lock_path: None,
    };

    let error = run_guarded_async("terminal", ScriptMutability::Immutable, &config, || {
        attempts.set(attempts.get() + 1);
        Err::<(), _>(anyhow!("relation not found"))
    })
    .await
    .unwrap_err();

    assert!(error.to_string().contains("relation not found"));
    assert_eq!(attempts.get(), 1);
}

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
