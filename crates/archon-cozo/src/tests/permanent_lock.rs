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
