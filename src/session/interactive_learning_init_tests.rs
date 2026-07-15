use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::interactive_learning_init::initialize_with;

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
