//! A socket produces a real permanent OS lock error, not a busy string.
use super::*;
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicUsize, Ordering};

fn permanent_error(calls: &AtomicUsize, context: &str) -> Result<()> {
    let socket = std::os::unix::net::UnixStream::pair().unwrap().0;
    let file = std::fs::File::from(OwnedFd::from(socket));
    if calls.fetch_add(1, Ordering::SeqCst) == 0 {
        locking::with_file_write_lock(file, Path::new("socket.lock"), context, || {
            panic!("a socket must not acquire a file lock")
        })
    } else {
        // If the old classifier retries, terminate the fixture with the same
        // real syscall error, without the faulty wrapper. No hung test thread.
        let mut lock = fd_lock::RwLock::new(file);
        Err(lock.try_write().err().expect("socket flock fails").into())
    }
}
fn assert_permanent(error: anyhow::Error, calls: &AtomicUsize) {
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "permanent lock error was retried: {error:#}"
    );
    let io = error
        .chain()
        .find_map(|e| e.downcast_ref::<std::io::Error>())
        .expect("OS cause must survive");
    assert_ne!(
        io.kind(),
        std::io::ErrorKind::WouldBlock,
        "the fixture must remain a permanent lock error"
    );
    assert!(io.raw_os_error().is_some(), "must retain the real OS cause");
    assert!(StoreBusy::find(error.as_ref()).is_none());
}
#[test]
fn permanent_lock_failure_is_prompt_and_retains_the_os_cause() {
    let calls = AtomicUsize::new(0);
    let error = run_guarded(
        "lock",
        ScriptMutability::Immutable,
        &CozoGuardConfig::default(),
        || permanent_error(&calls, "lock"),
    )
    .unwrap_err();
    assert_permanent(error, &calls);
}
#[test]
fn permanent_lock_failure_is_not_contention_even_with_busy_context() {
    let calls = AtomicUsize::new(0);
    let error = run_guarded(
        "lock",
        ScriptMutability::Immutable,
        &CozoGuardConfig::default(),
        || permanent_error(&calls, "database is locked"),
    )
    .unwrap_err();
    assert_permanent(error, &calls);
}
#[tokio::test]
async fn permanent_async_lock_failure_is_prompt_and_retains_the_os_cause() {
    let calls = Arc::new(AtomicUsize::new(0));
    let worker_calls = Arc::clone(&calls);
    let error = run_guarded_async(
        "lock",
        ScriptMutability::Immutable,
        &CozoGuardConfig::default(),
        move || permanent_error(&worker_calls, "lock"),
    )
    .await
    .unwrap_err();
    assert_permanent(error, &calls);
}

/// A FIFO is a real path whose `flock` fails with ENOTSUP on Apple systems:
/// the same permanent error a filesystem without flock gives.
#[cfg(target_vendor = "apple")]
fn fifo_lock(temp: &tempfile::TempDir) -> PathBuf {
    let path = temp.path().join("store.db.archon-cozo-write.lock");
    let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: mkfifo reads a NUL-terminated path that lives for the call.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    path
}

#[cfg(target_vendor = "apple")]
fn assert_names_lock(error: &anyhow::Error, path: &Path, context: &str) {
    let rendered = format!("{error:#}");
    assert!(
        rendered.contains(&path.display().to_string()),
        "a permanent lock error must name the lock path: {rendered}"
    );
    assert!(
        rendered.contains(context),
        "a permanent lock error must name the operation: {rendered}"
    );
    let io = error
        .chain()
        .find_map(|e| e.downcast_ref::<std::io::Error>())
        .expect("OS cause must survive");
    assert!(io.raw_os_error().is_some(), "{rendered}");
    assert!(StoreBusy::find(error.as_ref()).is_none(), "{rendered}");
}

#[cfg(target_vendor = "apple")]
#[test]
fn blocking_permanent_lock_error_names_the_lock_path() {
    let temp = tempfile::tempdir().unwrap();
    let path = fifo_lock(&temp);
    let started = std::time::Instant::now();
    let error = crate::with_write_lock_blocking(&path, "reserve fixture hash", || {
        panic!("a FIFO must not acquire a file lock")
    })
    .map(|()| ())
    .unwrap_err();
    assert_names_lock(&error, &path, "reserve fixture hash");
    assert!(started.elapsed() < Duration::from_secs(5), "must not wait");
}

#[cfg(target_vendor = "apple")]
#[test]
fn resuming_permanent_lock_error_names_the_lock_path() {
    let temp = tempfile::tempdir().unwrap();
    let path = fifo_lock(&temp);
    let error = crate::with_write_lock_resuming(
        &path,
        "ensure fixture schema",
        DEFAULT_WRITE_LOCK_WAIT,
        || panic!("a FIFO must not acquire a file lock"),
    )
    .map(|()| ())
    .unwrap_err();
    assert_names_lock(&error, &path, "ensure fixture schema");
}

#[cfg(target_vendor = "apple")]
#[test]
fn queued_guarded_write_permanent_lock_error_names_the_lock_path() {
    let temp = tempfile::tempdir().unwrap();
    let path = fifo_lock(&temp);
    let config = CozoGuardConfig::default()
        .with_write_lock_path(&path)
        .with_write_lock_wait(DEFAULT_WRITE_LOCK_WAIT);
    let calls = AtomicUsize::new(0);
    let error = run_guarded("queued write", ScriptMutability::Mutable, &config, || {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap_err();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_names_lock(&error, &path, "queued write");
}

/// Portable: the queued acquisition loop itself, on a socket whose `flock`
/// fails permanently, returns at once and names the lock path.
#[test]
fn queued_acquisition_permanent_error_names_the_lock_path() {
    let socket = std::os::unix::net::UnixStream::pair().unwrap().0;
    let file = std::fs::File::from(OwnedFd::from(socket));
    let path = Path::new("/archon-fixture/socket.archon-cozo-write.lock");
    let mut pending =
        crate::acquire::AcquireWait::new(path, "socket fixture", DEFAULT_WRITE_LOCK_WAIT);
    let started = std::time::Instant::now();
    let error = crate::acquire::acquire_file_lock(file, &mut pending, || -> Result<()> {
        panic!("a socket must not acquire a file lock")
    })
    .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5), "must not wait");
    let rendered = format!("{error:#}");
    assert!(
        rendered.contains("socket fixture: Cozo write lock failed at /archon-fixture/socket"),
        "{rendered}"
    );
    assert!(StoreBusy::find(error.as_ref()).is_none(), "{rendered}");
}
