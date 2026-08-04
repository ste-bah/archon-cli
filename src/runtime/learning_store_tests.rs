use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Barrier, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use cozo::{DbInstance, ScriptMutability};

use super::{
    acquire_for_path, acquire_for_path_with, acquire_for_path_with_async, clear_for_tests,
};

/// How long a test waits for a thread it has already unblocked.
///
/// Every use is a hang guard, not an assertion about speed: the thing being
/// waited for has already been signalled, and the bound exists only so a
/// genuine deadlock fails the suite instead of hanging it forever.
///
/// It was 2 seconds, which is a claim about scheduler latency rather than about
/// correctness, and the claim was false. Under the full binary suite these
/// threads contend with ~1,570 other tests, and
/// `panicking_open_releases_waiters_and_allows_a_retry` failed 2 runs in 3 —
/// on the slower runs (55s and 52s) and not on the faster one (38s), which is
/// the signature of a load-sensitive deadline rather than a logic error.
///
/// The `Duration::from_millis` waits elsewhere in this file are deliberately
/// NOT this constant: those are negative assertions that something has *not*
/// happened yet, so their shortness is the point.
const THREAD_HANDOFF_DEADLINE: Duration = Duration::from_secs(60);

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn db_instance_is_send_and_sync() {
    assert_send_sync::<DbInstance>();
}

#[test]
fn parallel_same_path_coalesces_open_and_schema_initialization() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("learning-state.db");
    clear_for_tests(&path);
    let (open_count, ensure_count) = counters();
    let barrier = Arc::new(Barrier::new(3));

    let first = spawn_counted_acquire(
        path.clone(),
        Arc::clone(&barrier),
        Arc::clone(&open_count),
        Arc::clone(&ensure_count),
    );
    let second = spawn_counted_acquire(
        path.clone(),
        Arc::clone(&barrier),
        open_count.clone(),
        ensure_count.clone(),
    );
    barrier.wait();
    let first = first.join().expect("first thread panicked")?;
    let second = second.join().expect("second thread panicked")?;

    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(1, open_count.load(Ordering::SeqCst));
    assert_eq!(1, ensure_count.load(Ordering::SeqCst));
    clear_for_tests(&path);
    Ok(())
}

#[test]
fn distinct_paths_initialize_independently() -> Result<()> {
    let first_temp = tempfile::tempdir()?;
    let second_temp = tempfile::tempdir()?;
    let first_path = first_temp.path().join("learning-state.db");
    let second_path = second_temp.path().join("learning-state.db");
    clear_for_tests(&first_path);
    clear_for_tests(&second_path);
    let (open_count, ensure_count) = counters();

    let first = acquire_for_path_with(
        &first_path,
        counting_opener(open_count.clone(), ensure_count.clone()),
    )?;
    let second = acquire_for_path_with(
        &second_path,
        counting_opener(open_count.clone(), ensure_count.clone()),
    )?;

    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(2, open_count.load(Ordering::SeqCst));
    assert_eq!(2, ensure_count.load(Ordering::SeqCst));
    clear_for_tests(&first_path);
    clear_for_tests(&second_path);
    Ok(())
}

#[test]
fn concurrent_waiters_are_released_after_failed_open_and_retry_once() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("learning-state.db");
    clear_for_tests(&path);
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let attempts = Arc::new(AtomicUsize::new(0));
    let release_rx = Arc::new(Mutex::new(release_rx));
    let first_attempts = Arc::clone(&attempts);
    let first_release_rx = Arc::clone(&release_rx);
    let first_path = path.clone();

    let first = std::thread::spawn(move || {
        acquire_for_path_with(&first_path, move |path| {
            first_attempts.fetch_add(1, Ordering::SeqCst);
            started_tx.send(()).expect("announce first open");
            first_release_rx
                .lock()
                .expect("release channel lock")
                .recv()
                .expect("release failed open");
            Err(anyhow!("first open failed for {}", path.display()))
        })
    });
    started_rx
        .recv_timeout(THREAD_HANDOFF_DEADLINE)
        .expect("first open begins");

    let waiter_attempts = Arc::clone(&attempts);
    let waiter_path = path.clone();
    let waiter = std::thread::spawn(move || {
        acquire_for_path_with(&waiter_path, move |path| {
            waiter_attempts.fetch_add(1, Ordering::SeqCst);
            open_test_db(path)
        })
    });

    release_tx.send(())?;
    let first_error = match first.join().expect("first thread panicked") {
        Ok(_) => panic!("first open unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(first_error.to_string().contains("first open failed"));
    let db = waiter
        .join()
        .expect("waiter thread panicked")
        .expect("waiter retries after failed open");

    assert_eq!(2, attempts.load(Ordering::SeqCst));
    drop(db);
    clear_for_tests(&path);
    Ok(())
}

#[test]
fn panicking_open_releases_waiters_and_allows_a_retry() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("learning-state.db");
    clear_for_tests(&path);
    let (opening_tx, opening_rx) = mpsc::channel();
    let (panic_tx, panic_rx) = mpsc::channel();
    let panic_rx = Arc::new(Mutex::new(panic_rx));
    let (waiter_result_tx, waiter_result_rx) = mpsc::channel();
    let first_path = path.clone();
    let panic_rx_for_opener = Arc::clone(&panic_rx);

    let opener = std::thread::spawn(move || {
        acquire_for_path_with(&first_path, move |_| -> Result<Arc<DbInstance>> {
            opening_tx.send(()).expect("announce opening cache entry");
            panic_rx_for_opener
                .lock()
                .expect("panic channel lock")
                .recv()
                .expect("release panic opener");
            panic!("test opener panic");
        })
    });
    opening_rx
        .recv_timeout(THREAD_HANDOFF_DEADLINE)
        .expect("opener creates the opening cache entry");

    let waiter_path = path.clone();
    let waiter = std::thread::spawn(move || {
        let result = acquire_for_path_with(&waiter_path, open_test_db);
        waiter_result_tx
            .send(result)
            .expect("report waiter acquisition result");
    });

    panic_tx.send(())?;
    assert!(opener.join().is_err(), "opener panic must propagate");
    let db = waiter_result_rx
        .recv_timeout(THREAD_HANDOFF_DEADLINE)
        .expect("waiter is released after opener panic")?;
    waiter.join().expect("waiter thread completes");

    drop(db);
    clear_for_tests(&path);
    Ok(())
}
#[test]
fn distinct_paths_open_independently_without_waiting_for_each_other() -> Result<()> {
    let first_temp = tempfile::tempdir()?;
    let second_temp = tempfile::tempdir()?;
    let first_path = first_temp.path().join("learning-state.db");
    let second_path = second_temp.path().join("learning-state.db");
    clear_for_tests(&first_path);
    clear_for_tests(&second_path);
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));

    let first = spawn_overlapping_acquire(
        first_path.clone(),
        started_tx.clone(),
        Arc::clone(&release_rx),
    );
    started_rx
        .recv_timeout(THREAD_HANDOFF_DEADLINE)
        .expect("first distinct path begins opening");
    let second = spawn_overlapping_acquire(second_path.clone(), started_tx, release_rx);
    started_rx
        .recv_timeout(THREAD_HANDOFF_DEADLINE)
        .expect("second distinct path begins before either open completes");

    release_tx.send(())?;
    release_tx.send(())?;
    first.join().expect("first thread panicked")?;
    second.join().expect("second thread panicked")?;
    clear_for_tests(&first_path);
    clear_for_tests(&second_path);
    Ok(())
}

#[test]
fn failed_open_is_removed_so_a_later_acquisition_retries() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("learning-state.db");
    clear_for_tests(&path);
    let attempts = Arc::new(AtomicUsize::new(0));
    let retry_attempts = Arc::clone(&attempts);

    let error = match acquire_for_path_with(&path, move |path| {
        attempts.fetch_add(1, Ordering::SeqCst);
        Err(anyhow!("initial open failed for {}", path.display()))
    }) {
        Ok(_) => panic!("first acquisition should fail"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("initial open failed"));

    let retry_for_open = Arc::clone(&retry_attempts);
    let db = acquire_for_path_with(&path, move |path| {
        retry_for_open.fetch_add(1, Ordering::SeqCst);
        open_test_db(path)
    })?;

    assert_eq!(2, retry_attempts.load(Ordering::SeqCst));
    drop(db);
    clear_for_tests(&path);
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlinked_parent_and_real_parent_share_the_cache_entry() -> Result<()> {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir()?;
    let real_parent = temp.path().join("real");
    std::fs::create_dir_all(&real_parent)?;
    let alias_parent = temp.path().join("alias");
    symlink(&real_parent, &alias_parent)?;
    let real_path = real_parent.join("learning-state.db");
    let alias_path = alias_parent.join(".").join("learning-state.db");
    clear_for_tests(&real_path);
    let (open_count, ensure_count) = counters();

    let real = acquire_for_path_with(
        &real_path,
        counting_opener(open_count.clone(), ensure_count.clone()),
    )?;
    let alias = acquire_for_path_with(
        &alias_path,
        counting_opener(open_count.clone(), ensure_count.clone()),
    )?;

    assert!(Arc::ptr_eq(&real, &alias));
    assert_eq!(1, open_count.load(Ordering::SeqCst));
    assert_eq!(1, ensure_count.load(Ordering::SeqCst));
    clear_for_tests(&real_path);
    Ok(())
}

#[cfg(unix)]
#[test]
fn pipeline_schema_initialization_uses_registered_final_file_alias_lock() -> Result<()> {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir()?;
    let path = temp.path().join("learning-state.db");
    let alias = temp.path().join("learning-state-alias.db");
    clear_for_tests(&path);
    let database = acquire_for_path(&path)?;
    symlink(&path, &alias)?;

    let alias_lock = archon_cozo::write_lock_path_for_db(&alias);
    let (locked_tx, locked_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let holder = std::thread::spawn(move || {
        archon_cozo::with_write_lock(&alias_lock, "hold pipeline alias lock", || {
            locked_tx.send(()).expect("announce held alias lock");
            release_rx.recv().expect("release held alias lock");
            Ok(())
        })
    });
    locked_rx.recv_timeout(THREAD_HANDOFF_DEADLINE)?;

    let (result_tx, result_rx) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        result_tx
            .send(archon_pipeline::learning::schema::initialize_learning_schemas(&database))
            .expect("report pipeline schema result");
    });

    assert!(
        result_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "pipeline schema write bypassed registered alias lock",
    );
    release_tx.send(())?;
    holder.join().expect("lock holder thread")?;
    result_rx.recv_timeout(THREAD_HANDOFF_DEADLINE)??;
    writer.join().expect("schema writer thread");
    clear_for_tests(&path);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn async_acquisition_does_not_block_the_runtime_worker() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("learning-state.db");
    clear_for_tests(&path);
    let (opening_tx, opening_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (progress_tx, progress_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));

    let coordinator = std::thread::spawn(move || {
        opening_rx.recv_timeout(THREAD_HANDOFF_DEADLINE).unwrap();
        let progressed = progress_rx.recv_timeout(Duration::from_millis(250)).is_ok();
        release_tx.send(()).unwrap();
        result_tx.send(progressed).unwrap();
    });

    let acquire = acquire_for_path_with_async(&path, move |path| {
        opening_tx.send(()).unwrap();
        release_rx.lock().unwrap().recv().unwrap();
        open_test_db(path)
    });
    let progress = async move {
        progress_tx.send(()).unwrap();
    };
    let (database, ()) = tokio::join!(acquire, progress);

    coordinator.join().unwrap();
    database?;
    assert!(result_rx.recv().unwrap());
    clear_for_tests(&path);
    Ok(())
}

#[test]
fn real_store_acquisition_reuses_the_handle_and_exposes_governed_schema() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("learning-state.db");
    clear_for_tests(&path);

    let first = acquire_for_path(&path)?;
    let second = acquire_for_path(&path)?;
    let rows = first
        .run_script(
            "::relations",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|error| anyhow!("{error}"))?;
    let name_column = rows
        .headers
        .iter()
        .position(|header| header == "name")
        .expect("relation listing should include a name column");
    let relation_exists = rows.rows.iter().any(|row| {
        row[name_column]
            .get_str()
            .is_some_and(|name| name == "provider_runtime_events")
    });

    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(
        archon_cozo::guarded_config_for(&first).and_then(|config| config.write_lock_path),
        Some(archon_cozo::write_lock_path_for_db(&path)),
    );
    assert!(
        relation_exists,
        "provider_runtime_events relation should exist"
    );
    clear_for_tests(&path);
    Ok(())
}

fn spawn_counted_acquire(
    path: std::path::PathBuf,
    barrier: Arc<Barrier>,
    open_count: Arc<AtomicUsize>,
    ensure_count: Arc<AtomicUsize>,
) -> std::thread::JoinHandle<Result<Arc<DbInstance>>> {
    std::thread::spawn(move || {
        barrier.wait();
        acquire_for_path_with(&path, counting_opener(open_count, ensure_count))
    })
}

fn counters() -> (Arc<AtomicUsize>, Arc<AtomicUsize>) {
    (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)))
}

fn counting_opener(
    open_count: Arc<AtomicUsize>,
    ensure_count: Arc<AtomicUsize>,
) -> impl Fn(&Path) -> Result<Arc<DbInstance>> + Send + Sync + 'static {
    move |path| {
        open_count.fetch_add(1, Ordering::SeqCst);
        let db = open_test_db(path)?;
        ensure_count.fetch_add(1, Ordering::SeqCst);
        archon_learning::schema::ensure_learning_schema(&db)?;
        Ok(db)
    }
}

fn open_test_db(path: &Path) -> Result<Arc<DbInstance>> {
    archon_learning::cozo_guard::open_sqlite_guarded(
        path.to_str().unwrap(),
        "open runtime learning-store test db",
    )
}

fn spawn_overlapping_acquire(
    path: std::path::PathBuf,
    started_tx: mpsc::Sender<()>,
    release_rx: Arc<Mutex<mpsc::Receiver<()>>>,
) -> std::thread::JoinHandle<Result<Arc<DbInstance>>> {
    std::thread::spawn(move || {
        acquire_for_path_with(&path, move |path| {
            started_tx.send(()).expect("announce open");
            release_rx
                .lock()
                .expect("release channel lock")
                .recv()
                .expect("release open");
            open_test_db(path)
        })
    })
}
