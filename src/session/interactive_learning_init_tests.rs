use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use super::interactive_learning_init::{initialize_schemas, initialize_with};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_learning_initialization_opens_once_and_shares_db_instance() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let opens = Arc::new(AtomicUsize::new(0));
    let opens_for_open = Arc::clone(&opens);

    let databases = initialize_with(
        temp_dir.path(),
        move |path| {
            let opens = Arc::clone(&opens_for_open);
            async move {
                opens.fetch_add(1, Ordering::SeqCst);
                archon_learning::cozo_guard::open_sqlite_guarded_async(
                    &path,
                    "test interactive learning db",
                )
                .await
            }
        },
        super::interactive_learning_init::initialize_schemas,
    )
    .await;

    assert_eq!(opens.load(Ordering::SeqCst), 1, "database must open once");
    let pipeline = databases.pipeline.expect("pipeline schema available");
    let governed = databases.governed.expect("governed schema available");
    assert!(Arc::ptr_eq(&pipeline, &governed));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_learning_schema_initialization_runs_on_blocking_thread() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let runtime_thread = std::thread::current().id();
    let schema_thread = Arc::new(std::sync::Mutex::new(None));
    let schema_thread_for_init = Arc::clone(&schema_thread);

    let databases = initialize_with(
        temp_dir.path(),
        |path| async move {
            archon_learning::cozo_guard::open_sqlite_guarded_async(
                &path,
                "test interactive learning db",
            )
            .await
        },
        move |_working_dir, db| {
            *schema_thread_for_init.lock().expect("schema thread lock") =
                Some(std::thread::current().id());
            super::interactive_learning_init::initialize_schemas(_working_dir, db)
        },
    )
    .await;

    assert!(databases.pipeline.is_some());
    assert!(databases.governed.is_some());
    assert_ne!(
        schema_thread
            .lock()
            .expect("schema thread lock")
            .expect("schema initializer called"),
        runtime_thread,
        "schema initialization must not run on the async runtime thread"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_learning_partial_schema_failure_retains_the_healthy_role() {
    let temp_dir = tempfile::tempdir().expect("temp dir");

    let databases = initialize_with(
        temp_dir.path(),
        |path| async move {
            archon_learning::cozo_guard::open_sqlite_guarded_async(
                &path,
                "test interactive learning db",
            )
            .await
        },
        |_working_dir, _db| super::interactive_learning_init::SchemaInitialization {
            pipeline: false,
            governed: true,
        },
    )
    .await;

    assert!(databases.pipeline.is_none());
    assert!(databases.governed.is_some());
}

#[test]
fn interactive_learning_schema_initialization_waits_for_held_sidecar_lock() {
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

#[tracing_test::traced_test]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_learning_schema_join_failure_warns_and_disables_persistence() {
    let temp_dir = tempfile::tempdir().expect("temp dir");

    let databases = initialize_with(
        temp_dir.path(),
        |path| async move {
            archon_learning::cozo_guard::open_sqlite_guarded_async(
                &path,
                "test interactive learning db",
            )
            .await
        },
        |_working_dir, _db| -> super::interactive_learning_init::SchemaInitialization {
            panic!("test blocking task panic")
        },
    )
    .await;

    assert!(databases.pipeline.is_none());
    assert!(databases.governed.is_none());
    assert!(logs_contain(
        "interactive learning schema initialization join failed"
    ));
}
