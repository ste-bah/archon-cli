//! The exit drain waits for pending teardown and sealing work, within a
//! no-progress bound (#297 round 9).
use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The pending set is process-wide: these tests take turns with it.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|error| error.into_inner())
}

#[test]
fn the_drain_waits_for_pending_work_to_end() {
    let _serial = serial();
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    let work = PendingWork::begin();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        flag.store(true, Ordering::SeqCst);
        drop(work);
    });
    assert!(drain(Duration::from_secs(5), Duration::from_secs(20)));
    assert!(done.load(Ordering::SeqCst), "the drain returned first");
    worker.join().unwrap();
}

#[test]
fn the_drain_gives_up_on_work_that_makes_no_progress() {
    let _serial = serial();
    let work = PendingWork::begin();
    let begun = Instant::now();
    assert!(!drain(Duration::from_millis(50), Duration::from_secs(2)));
    assert!(
        begun.elapsed() < Duration::from_secs(3),
        "{:?}",
        begun.elapsed()
    );
    drop(work);
}

#[test]
fn progress_keeps_the_drain_waiting_past_the_no_progress_bound() {
    let _serial = serial();
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    let work = PendingWork::begin();
    let worker = std::thread::spawn(move || {
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(40));
            work.progressed();
        }
        flag.store(true, Ordering::SeqCst);
        drop(work);
    });
    assert!(drain(Duration::from_millis(150), Duration::from_secs(20)));
    assert!(
        done.load(Ordering::SeqCst),
        "the drain gave up on live work"
    );
    worker.join().unwrap();
}

/// A process that exits while work is pending finishes the work first.
#[test]
fn process_exit_drains_pending_work() {
    const FLAG: &str = "ARCHON_TEST_HOST_EXIT_DRAIN";
    if let Some(path) = std::env::var_os(FLAG) {
        let work = PendingWork::begin();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            std::fs::write(path, "finished").unwrap();
            drop(work);
        });
        std::process::exit(0);
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("finished");
    let status = archon_shell::spawn::command(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "command::workflow_host_exit_drain::tests::process_exit_drains_pending_work",
        ])
        .env(FLAG, &path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "{status:?}");
    assert!(path.exists(), "the exit abandoned pending work");
}
