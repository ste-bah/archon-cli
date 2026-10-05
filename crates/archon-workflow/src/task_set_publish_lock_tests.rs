//! Issue 336: readers share the publish lock, writers hold it alone, and the
//! same-thread rules of Issue 294 still hold.

use std::sync::mpsc::channel;
use std::time::Duration;

use super::*;

const BLOCKED: Duration = Duration::from_millis(300);
const FINISHES: Duration = Duration::from_secs(20);

fn pin() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let pin = temp.path().join("pins/set.json");
    std::fs::create_dir_all(pin.parent().unwrap()).unwrap();
    (temp, pin)
}

#[test]
fn two_readers_hold_the_lock_at_once() {
    let (_temp, pin) = pin();
    let first = PublishLockFile::hold_shared(&pin).unwrap().unwrap();
    assert_eq!(first.mode(), LockMode::Shared);
    let (tx, rx) = channel();
    let other = pin.clone();
    let reader = std::thread::spawn(move || {
        let second = PublishLockFile::hold_shared(&other).unwrap().unwrap();
        tx.send(second.mode()).unwrap();
    });
    assert_eq!(
        rx.recv_timeout(FINISHES)
            .expect("a second reader was held back by the first"),
        LockMode::Shared
    );
    reader.join().unwrap();
    drop(first);
}

#[test]
fn a_writer_waits_for_a_reader_and_a_reader_for_a_writer() {
    let (_temp, pin) = pin();
    let read = PublishLockFile::hold_shared(&pin).unwrap().unwrap();
    let (tx, rx) = channel();
    let other = pin.clone();
    let writer = std::thread::spawn(move || {
        let write = PublishLockFile::hold(&other, None).unwrap().unwrap();
        tx.send(()).unwrap();
        std::thread::sleep(BLOCKED);
        drop(write);
    });
    assert!(
        rx.recv_timeout(BLOCKED).is_err(),
        "a writer ran beside a reader"
    );
    drop(read);
    rx.recv_timeout(FINISHES)
        .expect("the writer ran once the read ended");
    let (tx, rx) = channel();
    let other = pin.clone();
    let reader = std::thread::spawn(move || {
        let _read = PublishLockFile::hold_shared(&other).unwrap().unwrap();
        tx.send(()).unwrap();
    });
    writer.join().unwrap();
    rx.recv_timeout(FINISHES)
        .expect("the reader ran once the write ended");
    reader.join().unwrap();
}

#[test]
fn same_thread_nesting_reads_under_any_holder_and_refuses_a_write_inside_a_read() {
    let (_temp, pin) = pin();
    let read = PublishLockFile::hold_shared(&pin).unwrap().unwrap();
    assert!(PublishLockFile::hold_shared(&pin).unwrap().is_none());
    let refused = PublishLockFile::hold(&pin, None).err().unwrap();
    assert!(refused.contains("a write inside that read"), "{refused}");
    assert!(PublishLockFile::acquire(&lock_path(&pin)).is_err());
    drop(read);
    let write = PublishLockFile::hold(&pin, None).unwrap().unwrap();
    assert!(PublishLockFile::hold(&pin, None).unwrap().is_none());
    assert!(PublishLockFile::hold_shared(&pin).unwrap().is_none());
    assert!(PublishLockFile::acquire_shared(&lock_path(&pin)).is_err());
    drop(write);
    assert!(!held_here(&lock_path(&pin)));
}

#[test]
fn a_reader_settles_a_left_journal_under_the_exclusive_lock_then_reads_shared() {
    let (_temp, pin) = pin();
    let [journal, _] = journal_paths(&pin);
    std::fs::write(&journal, b"{}").unwrap();
    let lock = lock_path(&pin);
    let mut seen = Vec::new();
    let read = PublishLockFile::acquire_shared_settled(
        &pin,
        || {
            seen.push(held_mode_here(&lock));
            std::fs::remove_file(&journal).map_err(|error| error.to_string())
        },
        |error| error,
        |exhausted| exhausted,
    )
    .unwrap();
    assert_eq!(seen, [Some(LockMode::Exclusive)]);
    assert_eq!(read.mode(), LockMode::Shared);
}

#[test]
fn a_reader_whose_settlement_fails_reports_it_and_retries_on_the_next_read() {
    let (_temp, pin) = pin();
    let [journal, _] = journal_paths(&pin);
    std::fs::write(&journal, b"{}").unwrap();
    let failed = PublishLockFile::acquire_shared_settled(
        &pin,
        || Err("cleanup failed".to_string()),
        |error| error,
        |exhausted| exhausted,
    );
    assert_eq!(failed.err().as_deref(), Some("cleanup failed"));
    assert!(!held_here(&lock_path(&pin)), "a failed read kept the lock");
    let mut calls = 0;
    let read = PublishLockFile::acquire_shared_settled(
        &pin,
        || {
            calls += 1;
            std::fs::remove_file(&journal).map_err(|error| error.to_string())
        },
        |error| error,
        |exhausted| exhausted,
    );
    assert!(read.is_ok());
    assert_eq!(calls, 1, "the next read retried the settlement");
}

#[test]
fn a_repin_with_no_settlement_installed_refuses_a_left_journal() {
    let (_temp, pin) = pin();
    std::fs::write(&journal_paths(&pin)[1], b"{}").unwrap();
    // No host is linked into this crate's tests, so nothing is installed.
    let refused = PublishLockFile::hold(&pin, Some(Path::new("/tasks")))
        .err()
        .unwrap();
    assert!(refused.contains("was interrupted"), "{refused}");
    assert!(!held_here(&lock_path(&pin)));
}

#[test]
fn a_reader_whose_settlements_run_out_reports_it_as_exhausted() {
    let (_temp, pin) = pin();
    let [journal, _] = journal_paths(&pin);
    std::fs::write(&journal, b"{}").unwrap();
    let mut calls = 0;
    let failed = PublishLockFile::acquire_shared_settled(
        &pin,
        || {
            // Each settlement "succeeds" but the journal is left again.
            calls += 1;
            Ok::<(), String>(())
        },
        |error| format!("lock: {error}"),
        |exhausted| format!("exhausted: {exhausted}"),
    );
    let error = failed.err().unwrap();
    assert!(error.starts_with("exhausted: "), "{error}");
    assert!(error.contains("3 attempts"), "{error}");
    assert_eq!(calls, READ_ATTEMPTS);
    assert!(!held_here(&lock_path(&pin)));
}

#[test]
fn a_reader_whose_last_settlement_succeeds_reads_on() {
    let (_temp, pin) = pin();
    let [journal, _] = journal_paths(&pin);
    std::fs::write(&journal, b"{}").unwrap();
    let mut calls = 0;
    let read = PublishLockFile::acquire_shared_settled(
        &pin,
        || {
            calls += 1;
            if calls == READ_ATTEMPTS {
                std::fs::remove_file(&journal).map_err(|error| error.to_string())?;
            }
            Ok(())
        },
        |error| error,
        |exhausted| exhausted,
    )
    .expect("the read after the last settlement goes on");
    assert_eq!(calls, READ_ATTEMPTS);
    assert_eq!(read.mode(), LockMode::Shared);
}

#[test]
fn of_two_readers_that_find_one_journal_only_one_settles_it() {
    let (_temp, pin) = pin();
    let [journal, _] = journal_paths(&pin);
    std::fs::write(&journal, b"{}").unwrap();
    // Both readers queue behind a writer, so both find the journal.
    let writer = PublishLockFile::acquire(&lock_path(&pin)).unwrap();
    let settled = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let readers: Vec<_> = (0..2)
        .map(|_| {
            let (pin, journal) = (pin.clone(), journal.clone());
            let (settled, barrier) = (settled.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                let read = PublishLockFile::acquire_shared_settled(
                    &pin,
                    || {
                        settled.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(100));
                        std::fs::remove_file(&journal).map_err(|error| error.to_string())
                    },
                    |error| error,
                    |exhausted| exhausted,
                );
                read.map(|read| read.mode())
            })
        })
        .collect();
    barrier.wait();
    std::thread::sleep(BLOCKED);
    drop(writer);
    for reader in readers {
        assert_eq!(reader.join().unwrap(), Ok(LockMode::Shared));
    }
    assert_eq!(
        settled.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the second reader settled a journal the first had settled"
    );
}
