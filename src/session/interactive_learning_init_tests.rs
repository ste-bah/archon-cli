use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use super::interactive_learning_init::{
    BlockingInitialization, SchemaInitialization, initialize, initialize_governed_schemas,
    initialize_schemas, initialize_with,
};

fn open_and_initialize(working_dir: std::path::PathBuf) -> Result<BlockingInitialization> {
    let db_path = crate::command::store_paths::learning_db_path_for_dir(&working_dir);
    let config = archon_cozo::CozoGuardConfig::for_interactive_db_path(&db_path);
    let db = Arc::new(archon_cozo::open_sqlite_guarded(
        &db_path.to_string_lossy(),
        "test interactive learning db",
        &config,
    )?);
    let schemas = initialize_schemas(&working_dir, db.as_ref());
    Ok(BlockingInitialization { db, schemas })
}

#[tokio::test(flavor = "current_thread")]
async fn interactive_learning_initialization_keeps_current_thread_runtime_responsive_while_open_blocks()
 {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let (open_started_tx, open_started_rx) = mpsc::channel();
    let (release_open_tx, release_open_rx) = mpsc::channel();
    let (progress_tx, progress_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();

    let coordinator = std::thread::spawn(move || {
        open_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("open boundary entered");
        let progressed = progress_rx.recv_timeout(Duration::from_millis(250)).is_ok();
        release_open_tx.send(()).expect("release open boundary");
        result_tx.send(progressed).expect("report runtime progress");
    });

    let initialization = initialize_with(temp_dir.path(), move |working_dir| {
        open_started_tx.send(()).expect("announce open boundary");
        release_open_rx.recv().expect("release open boundary");
        open_and_initialize(working_dir)
    });
    let progress = async move {
        let _ = progress_tx.send(());
    };
    let (databases, ()) = tokio::join!(initialization, progress);

    coordinator.join().expect("coordinator joins");

    assert!(
        result_rx.recv().expect("runtime progress result"),
        "another Tokio task must progress while the database open is held"
    );
    assert!(databases.pipeline.is_some());
    assert!(databases.governed.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_learning_initialization_reuses_runtime_cache_handle() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let db_path = crate::command::store_paths::learning_db_path_for_dir(temp_dir.path());
    crate::runtime::learning_store::clear_for_tests(&db_path);
    let cached = crate::runtime::learning_store::acquire_for_dir(temp_dir.path())
        .expect("cached learning store");

    let databases = initialize(temp_dir.path()).await;

    let pipeline = databases.pipeline.expect("pipeline schema available");
    let governed = databases.governed.expect("governed schema available");
    assert!(Arc::ptr_eq(&cached, &pipeline));
    assert!(Arc::ptr_eq(&cached, &governed));
    crate::runtime::learning_store::clear_for_tests(&db_path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_learning_initialization_opens_once_and_shares_db_instance() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let opens = Arc::new(AtomicUsize::new(0));
    let opens_for_init = Arc::clone(&opens);

    let databases = initialize_with(temp_dir.path(), move |working_dir| {
        opens_for_init.fetch_add(1, Ordering::SeqCst);
        open_and_initialize(working_dir)
    })
    .await;

    assert_eq!(opens.load(Ordering::SeqCst), 1, "database must open once");
    let pipeline = databases.pipeline.expect("pipeline schema available");
    let governed = databases.governed.expect("governed schema available");
    assert!(Arc::ptr_eq(&pipeline, &governed));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_learning_initialization_runs_on_blocking_thread() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let runtime_thread = std::thread::current().id();
    let initialization_thread = Arc::new(std::sync::Mutex::new(None));
    let initialization_thread_for_init = Arc::clone(&initialization_thread);

    let databases = initialize_with(temp_dir.path(), move |working_dir| {
        *initialization_thread_for_init
            .lock()
            .expect("initialization thread lock") = Some(std::thread::current().id());
        open_and_initialize(working_dir)
    })
    .await;

    assert!(databases.pipeline.is_some());
    assert!(databases.governed.is_some());
    assert_ne!(
        initialization_thread
            .lock()
            .expect("initialization thread lock")
            .expect("initializer called"),
        runtime_thread,
        "database and schema initialization must not run on the async runtime thread"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_learning_partial_schema_failure_retains_the_healthy_role() {
    let temp_dir = tempfile::tempdir().expect("temp dir");

    let databases = initialize_with(temp_dir.path(), move |working_dir| {
        let db_path = crate::command::store_paths::learning_db_path_for_dir(&working_dir);
        let config = archon_cozo::CozoGuardConfig::for_interactive_db_path(&db_path);
        let db = Arc::new(archon_cozo::open_sqlite_guarded(
            &db_path.to_string_lossy(),
            "test interactive learning db",
            &config,
        )?);
        Ok(BlockingInitialization {
            db,
            schemas: SchemaInitialization {
                pipeline: false,
                governed: true,
            },
        })
    })
    .await;

    assert!(databases.pipeline.is_none());
    assert!(databases.governed.is_some());
}

#[test]
fn interactive_learning_schema_initialization_waits_for_held_sidecar_lock() {
    let _learning_db_env_guard = crate::command::store_paths::LEARNING_DB_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let db_path = crate::command::store_paths::learning_db_path_for_dir(temp_dir.path());
    let db = archon_learning::cozo_guard::open_sqlite_guarded(
        &db_path.to_string_lossy(),
        "test interactive learning db",
    )
    .expect("open learning db");
    let lock_path = archon_cozo::write_lock_path_for_db(&db_path);
    let (locked_tx, locked_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let lock_holder = std::thread::spawn(move || {
        archon_cozo::with_write_lock(&lock_path, "test lock holder", || {
            locked_tx.send(()).expect("announce held lock");
            release_rx.recv().expect("release held lock");
            Ok(())
        })
    });
    locked_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("sidecar lock acquired");

    let (result_tx, result_rx) = mpsc::channel();
    let working_dir = temp_dir.path().to_owned();
    let initialization = std::thread::spawn(move || {
        result_tx
            .send(initialize_schemas(&working_dir, &db))
            .expect("report schema initialization");
    });

    let completed_before_release = result_rx.recv_timeout(Duration::from_secs(2)).is_ok();
    release_tx.send(()).expect("release lock");
    lock_holder
        .join()
        .expect("lock holder joins")
        .expect("lock holder succeeds");

    assert!(
        !completed_before_release,
        "pipeline schemas must wait for the held sidecar lock"
    );
    let result = result_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("schema initialization after lock release");
    initialization.join().expect("schema thread joins");
    assert!(
        result.pipeline,
        "pipeline schemas initialize after lock release"
    );
    assert!(
        result.governed,
        "governed schemas remain independently available"
    );
}

#[test]
fn interactive_governed_schema_initialization_waits_for_held_sidecar_lock() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let db_path = crate::command::store_paths::learning_db_path_for_dir(temp_dir.path());
    let db = archon_learning::cozo_guard::open_sqlite_guarded(
        &db_path.to_string_lossy(),
        "test interactive learning db",
    )
    .expect("open learning db");
    let lock_path = archon_cozo::write_lock_path_for_db(&db_path);
    let (locked_tx, locked_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let lock_holder = std::thread::spawn(move || {
        archon_cozo::with_write_lock(&lock_path, "test lock holder", || {
            locked_tx.send(()).expect("announce held lock");
            release_rx.recv().expect("release held lock");
            Ok(())
        })
    });
    locked_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("sidecar lock acquired");

    let (result_tx, result_rx) = mpsc::channel();
    let config = archon_cozo::CozoGuardConfig::for_db_path(&db_path);
    let initialization = std::thread::spawn(move || {
        result_tx
            .send(initialize_governed_schemas(&db, &config))
            .expect("report governed schema initialization");
    });

    let completed_before_release = result_rx.recv_timeout(Duration::from_millis(100)).is_ok();
    release_tx.send(()).expect("release lock");
    lock_holder
        .join()
        .expect("lock holder joins")
        .expect("lock holder succeeds");

    assert!(
        !completed_before_release,
        "governed schemas must wait for the held same-path sidecar lock"
    );
    assert!(
        result_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("governed schema initialization after lock release"),
        "governed schemas initialize after lock release"
    );
    initialization.join().expect("schema thread joins");
}

#[tracing_test::traced_test]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_learning_open_failure_warns_about_both_disabled_roles() {
    let temp_dir = tempfile::tempdir().expect("temp dir");

    let databases = initialize_with(temp_dir.path(), move |_| {
        Err(anyhow::anyhow!("test interactive learning db open failure"))
    })
    .await;

    assert!(databases.pipeline.is_none());
    assert!(databases.governed.is_none());
    assert!(logs_contain(
        "pipeline persistence and governed runtime evidence disabled"
    ));
}

#[tracing_test::traced_test]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_learning_schema_join_failure_warns_about_both_disabled_roles() {
    let temp_dir = tempfile::tempdir().expect("temp dir");

    let databases = initialize_with(temp_dir.path(), move |working_dir| {
        let _ = working_dir;
        panic!("test blocking task panic")
    })
    .await;

    assert!(databases.pipeline.is_none());
    assert!(databases.governed.is_none());
    assert!(logs_contain(
        "pipeline persistence and governed runtime evidence disabled"
    ));
}
