//! A wedged lock holder ends a resuming acquisition as one typed pause.
//!
//! The resuming acquisition is the production path for document schema
//! creation and content-hash reservation. It must keep waiting while the
//! holder makes progress, and it must return `StoreBusy` once one full
//! no-progress window passes. It must never restart that window by itself.
use archon_cozo::{
    CozoGuardConfig, StoreBusy, run_script_guarded, with_write_lock_blocking_timeout,
    with_write_lock_resuming,
};
use cozo::{DbInstance, ScriptMutability};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_millis(400);
/// A correct acquisition returns about one window after the last progress.
/// An acquisition that restarts its window never returns while the holder is
/// wedged, so the test gives up here instead of hanging.
const GIVE_UP: Duration = Duration::from_secs(10);

struct Store {
    _temp: tempfile::TempDir,
    db_path: PathBuf,
    lock: PathBuf,
}

fn store() -> Store {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("stalled.db");
    let lock = archon_cozo::write_lock_path_for_db(&db_path);
    let db = DbInstance::new("sqlite", db_path.to_str().unwrap(), "").unwrap();
    db.run_script(
        ":create progress { key: Int => value: Int }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    Store {
        _temp: temp,
        db_path,
        lock,
    }
}

/// Run the resuming acquisition on its own thread and report when it returned.
fn resume(lock: &Path) -> mpsc::Receiver<(anyhow::Result<&'static str>, Instant, bool)> {
    let (sender, receiver) = mpsc::channel();
    let lock = lock.to_path_buf();
    std::thread::spawn(move || {
        let mut entered = false;
        let result = with_write_lock_resuming(&lock, "resuming waiter", WINDOW, || {
            entered = true;
            Ok("entered")
        });
        let _ = sender.send((result, Instant::now(), entered));
    });
    receiver
}

fn assert_pause(result: anyhow::Result<&'static str>, entered: bool, lock: &Path) {
    let error = result.expect_err("a wedged holder must end the wait as a pause");
    assert!(!entered, "the body must not run without the lock");
    let busy = StoreBusy::find(error.as_ref()).expect("the pause must stay typed StoreBusy");
    assert!(
        busy.detail.contains(&lock.display().to_string()),
        "the pause must name the lock path: {busy}"
    );
}

#[test]
fn resuming_acquisition_pauses_when_a_file_lock_holder_is_wedged() {
    let store = store();
    let mut held = fd_lock::RwLock::new(std::fs::File::create(&store.lock).unwrap());
    let _guard = held.try_write().unwrap();
    let started = Instant::now();
    let (result, returned, entered) = resume(&store.lock)
        .recv_timeout(GIVE_UP)
        .expect("resuming acquisition never returned while the holder was wedged");
    assert_pause(result, entered, &store.lock);
    assert!(
        returned.duration_since(started) >= WINDOW,
        "paused before one full no-progress window"
    );
}

#[test]
fn resuming_acquisition_pauses_when_a_process_lock_holder_is_wedged() {
    let store = store();
    let (held, holding) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let lock = store.lock.clone();
    let holder = std::thread::spawn(move || {
        with_write_lock_blocking_timeout(&lock, "wedged holder", WINDOW, || {
            held.send(()).unwrap();
            let _ = released.recv();
            Ok(())
        })
    });
    holding.recv_timeout(GIVE_UP).unwrap();
    let outcome = resume(&store.lock).recv_timeout(GIVE_UP);
    release.send(()).unwrap();
    holder.join().unwrap().unwrap();
    let (result, _, entered) =
        outcome.expect("resuming acquisition never returned while this process held the lock");
    assert_pause(result, entered, &store.lock);
}

#[test]
fn resuming_acquisition_waits_through_progress_then_pauses_one_window_after_it_stops() {
    let store = store();
    let (held, holding) = mpsc::channel();
    let (last, written) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let lock = store.lock.clone();
    let db_path = store.db_path.clone();
    let holder = std::thread::spawn(move || {
        let db = DbInstance::new("sqlite", db_path.to_str().unwrap(), "").unwrap();
        let config = CozoGuardConfig::for_db_path(&db_path);
        with_write_lock_blocking_timeout(&lock, "progressing holder", WINDOW, || {
            held.send(()).unwrap();
            // Real committed writes, three times as long as the window.
            for value in 0..12 {
                run_script_guarded(
                    &db,
                    &format!("?[key, value] <- [[1, {value}]] :put progress {{key => value}}"),
                    Default::default(),
                    ScriptMutability::Mutable,
                    "holder progress",
                    &config,
                )
                .unwrap();
                std::thread::sleep(Duration::from_millis(100));
            }
            last.send(Instant::now()).unwrap();
            let _ = released.recv();
            Ok(())
        })
    });
    holding.recv_timeout(GIVE_UP).unwrap();
    let started = Instant::now();
    let outcome = resume(&store.lock).recv_timeout(GIVE_UP + Duration::from_secs(2));
    release.send(()).unwrap();
    holder.join().unwrap().unwrap();
    let (result, returned, entered) =
        outcome.expect("resuming acquisition never returned after the holder stopped");
    assert_pause(result, entered, &store.lock);
    let last_write = written.recv().unwrap() - Duration::from_millis(100);
    assert!(
        returned.duration_since(started) >= Duration::from_millis(1_000),
        "progress must keep the acquisition waiting"
    );
    assert!(
        returned.duration_since(last_write) >= WINDOW,
        "paused before a full window without progress"
    );
}

#[test]
fn resuming_acquisition_enters_when_a_progressing_holder_releases() {
    let store = store();
    let (held, holding) = mpsc::channel();
    let lock = store.lock.clone();
    let db_path = store.db_path.clone();
    let holder = std::thread::spawn(move || {
        let db = DbInstance::new("sqlite", db_path.to_str().unwrap(), "").unwrap();
        let config = CozoGuardConfig::for_db_path(&db_path);
        with_write_lock_blocking_timeout(&lock, "progressing holder", WINDOW, || {
            held.send(()).unwrap();
            for value in 0..10 {
                run_script_guarded(
                    &db,
                    &format!("?[key, value] <- [[2, {value}]] :put progress {{key => value}}"),
                    Default::default(),
                    ScriptMutability::Mutable,
                    "holder progress",
                    &config,
                )
                .unwrap();
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(())
        })
    });
    holding.recv_timeout(GIVE_UP).unwrap();
    let (result, _, entered) = resume(&store.lock).recv_timeout(GIVE_UP).unwrap();
    holder.join().unwrap().unwrap();
    assert_eq!(result.unwrap(), "entered");
    assert!(entered);
}
