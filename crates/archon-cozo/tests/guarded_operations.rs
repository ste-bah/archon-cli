use std::panic;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use archon_cozo::{CozoGuardConfig, run_guarded};
use cozo::ScriptMutability;
use tempfile::TempDir;

const TEST_TIMEOUT: Duration = Duration::from_secs(2);
static PANIC_HOOK_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn overlap_count(
    mutability: ScriptMutability,
    configs: [CozoGuardConfig; 2],
    should_overlap: bool,
) -> usize {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let ready = Arc::new(Barrier::new(3));
    let go = Arc::new(Barrier::new(3));
    let handles: Vec<_> = configs
        .into_iter()
        .map(|config| {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            let entered_tx = entered_tx.clone();
            let release_rx = Arc::clone(&release_rx);
            let ready = Arc::clone(&ready);
            let go = Arc::clone(&go);
            thread::spawn(move || {
                ready.wait();
                go.wait();
                run_guarded("overlap test", mutability, &config, || {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(current, Ordering::SeqCst);
                    entered_tx.send(()).unwrap();
                    release_rx.lock().unwrap().recv().unwrap();
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                })
            })
        })
        .collect();
    drop(entered_tx);
    ready.wait();
    go.wait();

    entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    if should_overlap {
        entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    }
    release_tx.send(()).unwrap();
    release_tx.send(()).unwrap();
    for handle in handles {
        handle.join().unwrap().unwrap();
    }

    peak.load(Ordering::SeqCst)
}

#[test]
fn guarded_reads_overlap() {
    let peak = overlap_count(
        ScriptMutability::Immutable,
        [CozoGuardConfig::default(), CozoGuardConfig::default()],
        true,
    );

    assert_eq!(peak, 2);
}

#[test]
fn writes_with_different_lock_paths_overlap() {
    let temp = TempDir::new().unwrap();
    let peak = overlap_count(
        ScriptMutability::Mutable,
        [
            CozoGuardConfig::default().with_write_lock_path(temp.path().join("one.lock")),
            CozoGuardConfig::default().with_write_lock_path(temp.path().join("two.lock")),
        ],
        true,
    );

    assert_eq!(peak, 2);
}

#[test]
fn writes_with_the_same_lock_path_are_serialized() {
    let temp = TempDir::new().unwrap();
    let lock_path = temp.path().join("shared.lock");
    let peak = overlap_count(
        ScriptMutability::Mutable,
        [
            CozoGuardConfig::default().with_write_lock_path(&lock_path),
            CozoGuardConfig::default().with_write_lock_path(lock_path),
        ],
        false,
    );

    assert_eq!(peak, 1);
}

#[test]
fn unkeyed_writes_are_serialized() {
    let peak = overlap_count(
        ScriptMutability::Mutable,
        [CozoGuardConfig::default(), CozoGuardConfig::default()],
        false,
    );

    assert_eq!(peak, 1);
}

#[test]
fn guarded_panics_are_suppressed_and_unrelated_panics_use_the_installed_hook() {
    let _hook_guard = PANIC_HOOK_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap();
    let hook_calls = Arc::new(AtomicUsize::new(0));
    let hook_calls_for_hook = Arc::clone(&hook_calls);
    panic::set_hook(Box::new(move |_| {
        hook_calls_for_hook.fetch_add(1, Ordering::SeqCst);
    }));

    let guarded_panic = run_guarded(
        "guarded panic",
        ScriptMutability::Immutable,
        &CozoGuardConfig::default(),
        || -> Result<()> { panic!("guarded panic") },
    );
    assert!(
        guarded_panic
            .unwrap_err()
            .to_string()
            .contains("guarded panic")
    );
    assert_eq!(hook_calls.load(Ordering::SeqCst), 0);

    let (guarded_entered_tx, guarded_entered_rx) = mpsc::channel();
    let (guarded_release_tx, guarded_release_rx) = mpsc::channel();
    let guarded_operation = thread::spawn(move || {
        run_guarded(
            "active guard",
            ScriptMutability::Immutable,
            &CozoGuardConfig::default(),
            || {
                guarded_entered_tx.send(()).unwrap();
                guarded_release_rx.recv().unwrap();
                Ok(())
            },
        )
    });
    guarded_entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    let unrelated_panic = thread::spawn(|| panic!("unrelated panic"));
    assert!(unrelated_panic.join().is_err());
    assert_eq!(hook_calls.load(Ordering::SeqCst), 1);

    guarded_release_tx.send(()).unwrap();
    guarded_operation.join().unwrap().unwrap();

    let nested_panic = run_guarded(
        "outer guard",
        ScriptMutability::Immutable,
        &CozoGuardConfig::default(),
        || {
            let inner = run_guarded(
                "inner guard",
                ScriptMutability::Immutable,
                &CozoGuardConfig::default(),
                || -> Result<()> { panic!("inner panic") },
            );
            assert!(inner.is_err());
            panic!("outer panic");
            #[allow(unreachable_code)]
            Ok(())
        },
    );
    assert!(nested_panic.is_err());
    assert_eq!(hook_calls.load(Ordering::SeqCst), 1);
}
