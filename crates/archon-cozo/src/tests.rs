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
        "write-lock unavailable",
        "write lock unavailable",
    ] {
        assert!(is_retryable_cozo_error(message), "{message}");
    }

    for message in [
        "code 50",
        "code 500",
        "code: Some(50)",
        "code: Some(500)",
        "database is locked (code 500)",
        "database table is locked (code 500)",
        "poison error",
        "would-block",
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

    let (attempted_tx, attempted_rx) = tokio::sync::oneshot::channel();
    let mut attempted_tx = Some(attempted_tx);
    let retry = tokio::spawn(async move {
        run_guarded_async(
            "async retry",
            ScriptMutability::Immutable,
            &config,
            move || {
                let attempt = attempts_for_run.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if let Some(tx) = attempted_tx.take() {
                    let _ = tx.send(());
                }
                if attempt == 0 {
                    return Err(anyhow!("database is locked"));
                }
                Ok("success")
            },
        )
        .await
    });
    attempted_rx.await.unwrap();

    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    task.await.unwrap();
    assert!(yielded.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!retry.is_finished());

    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(retry.await.unwrap().unwrap(), "success");
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn async_guarded_operation_runs_off_runtime_worker() {
    use std::sync::mpsc;

    let (write_started_tx, write_started_rx) = mpsc::channel();
    let (release_write_tx, release_write_rx) = mpsc::channel();
    let (progress_tx, progress_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();

    let coordinator = std::thread::spawn(move || {
        write_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("guarded operation entered");
        let progressed = progress_rx.recv_timeout(Duration::from_millis(250)).is_ok();
        release_write_tx
            .send(())
            .expect("release guarded operation");
        result_tx.send(progressed).expect("report runtime progress");
    });

    let config = CozoGuardConfig {
        max_attempts: 1,
        initial_backoff: Duration::ZERO,
        max_backoff: Duration::ZERO,
        write_lock_path: None,
    };
    let guarded = run_guarded_async(
        "blocking guarded operation",
        ScriptMutability::Immutable,
        &config,
        move || {
            write_started_tx.send(()).expect("announce operation");
            release_write_rx.recv().expect("release operation");
            Ok(())
        },
    );
    let progress = async move {
        progress_tx.send(()).expect("report runtime progress");
    };
    let (result, ()) = tokio::join!(guarded, progress);

    coordinator.join().expect("coordinator joins");
    result.expect("guarded operation succeeds");
    assert!(
        result_rx.recv().expect("runtime progress result"),
        "another Tokio task must progress while guarded operation blocks"
    );
}

#[tokio::test(start_paused = true)]
async fn async_guarded_terminal_error_does_not_retry() {
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let attempts_for_run = Arc::clone(&attempts);
    let config = CozoGuardConfig {
        max_attempts: 2,
        initial_backoff: Duration::from_secs(1),
        max_backoff: Duration::from_secs(1),
        write_lock_path: None,
    };

    let error = run_guarded_async(
        "terminal",
        ScriptMutability::Immutable,
        &config,
        move || {
            attempts_for_run.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err::<(), _>(anyhow!("relation not found"))
        },
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("relation not found"));
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
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
