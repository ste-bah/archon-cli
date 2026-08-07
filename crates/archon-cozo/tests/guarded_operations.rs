use std::panic;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use archon_cozo::{CozoGuardConfig, GuardedDbInstance, run_guarded};
use cozo::{DbInstance, ScriptMutability};
use tempfile::TempDir;

const TEST_TIMEOUT: Duration = Duration::from_secs(2);
const PANIC_HOOK_CHILD_ENV: &str = "ARCHON_COZO_PANIC_HOOK_CHILD";

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
fn guarded_database_retains_its_write_lock_identity() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("bound.db");
    let config = CozoGuardConfig::for_db_path(&db_path);
    let expected_lock = config.write_lock_path.clone();
    let database = GuardedDbInstance::new(DbInstance::new("mem", "", "").unwrap(), config);

    assert_eq!(database.config().write_lock_path, expected_lock);
}

#[test]
fn guarded_database_runs_scripts_with_its_bound_config() {
    let temp = TempDir::new().unwrap();
    let database = GuardedDbInstance::new(
        DbInstance::new("mem", "", "").unwrap(),
        CozoGuardConfig::for_db_path(temp.path().join("bound.db")),
    );

    let result = database
        .run_script_guarded(
            "?[value] <- [[1]]",
            Default::default(),
            ScriptMutability::Immutable,
            "bound database query",
        )
        .unwrap();

    assert_eq!(result.rows.len(), 1);
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

#[cfg(unix)]
#[test]
fn writes_through_symlinked_parent_are_serialized_after_lock_creation() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let real_parent = temp.path().join("real");
    let alias_parent = temp.path().join("alias");
    std::fs::create_dir(&real_parent).unwrap();
    symlink(&real_parent, &alias_parent).unwrap();

    let real_path = real_parent.join("shared.lock");
    let alias_path = alias_parent.join("shared.lock");
    let config = CozoGuardConfig {
        max_attempts: 1,
        initial_backoff: Duration::ZERO,
        max_backoff: Duration::ZERO,
        write_lock_path: Some(alias_path),
        write_lock_wait: None,
    };
    let (first_entered_tx, first_entered_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let first = thread::spawn(move || {
        run_guarded("symlinked lock", ScriptMutability::Mutable, &config, || {
            first_entered_tx.send(()).unwrap();
            release_first_rx.recv().unwrap();
            Ok(())
        })
    });
    first_entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    let (second_entered_tx, second_entered_rx) = mpsc::channel();
    let second = thread::spawn(move || {
        run_guarded(
            "real lock",
            ScriptMutability::Mutable,
            &CozoGuardConfig {
                max_attempts: 1,
                initial_backoff: Duration::ZERO,
                max_backoff: Duration::ZERO,
                write_lock_path: Some(real_path),
                write_lock_wait: None,
            },
            || {
                second_entered_tx.send(()).unwrap();
                Ok(())
            },
        )
    });

    assert!(
        second_entered_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err(),
        "real-path write bypassed the mutex"
    );
    release_first_tx.send(()).unwrap();
    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();
}

#[cfg(unix)]
#[test]
fn writes_through_final_file_symlinks_are_serialized() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let real_db = temp.path().join("real.db");
    let alias_db = temp.path().join("alias.db");
    std::fs::File::create(&real_db).unwrap();
    symlink(&real_db, &alias_db).unwrap();

    let peak = overlap_count(
        ScriptMutability::Mutable,
        [
            CozoGuardConfig::for_db_path(&real_db),
            CozoGuardConfig::for_db_path(&alias_db),
        ],
        false,
    );

    assert_eq!(peak, 1);
}

#[cfg(unix)]
#[test]
fn writes_through_dangling_final_file_symlinks_are_serialized() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let real_db = temp.path().join("real.db");
    let alias_db = temp.path().join("alias.db");
    symlink(&real_db, &alias_db).unwrap();

    let first_config = CozoGuardConfig::for_db_path(&real_db);
    let second_config = CozoGuardConfig::for_db_path(&alias_db);
    let (first_entered_tx, first_entered_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let first = thread::spawn(move || {
        run_guarded(
            "real dangling-symlink target",
            ScriptMutability::Mutable,
            &first_config,
            || {
                first_entered_tx.send(()).unwrap();
                release_first_rx.recv().unwrap();
                Ok(())
            },
        )
    });
    first_entered_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    let (second_ready_tx, second_ready_rx) = mpsc::channel();
    let (second_entered_tx, second_entered_rx) = mpsc::channel();
    let second = thread::spawn(move || {
        second_ready_tx.send(()).unwrap();
        run_guarded(
            "dangling final-file alias",
            ScriptMutability::Mutable,
            &second_config,
            || {
                second_entered_tx.send(()).unwrap();
                Ok(())
            },
        )
    });
    second_ready_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    assert!(
        second_entered_rx
            .recv_timeout(Duration::from_millis(250))
            .is_err(),
        "dangling final-file alias bypassed the target database lock"
    );
    release_first_tx.send(()).unwrap();
    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();
}

#[test]
fn nested_mutable_same_path_guard_does_not_deadlock() {
    let temp = TempDir::new().unwrap();
    let config = CozoGuardConfig::for_db_path(temp.path().join("nested.db"));
    let inner_config = config.clone();
    let (result_tx, result_rx) = mpsc::channel();

    thread::spawn(move || {
        let result = run_guarded("outer write", ScriptMutability::Mutable, &config, || {
            run_guarded(
                "inner write",
                ScriptMutability::Mutable,
                &inner_config,
                || Ok(()),
            )
        });
        result_tx.send(result).unwrap();
    });

    result_rx
        .recv_timeout(TEST_TIMEOUT)
        .expect("nested same-path write guard deadlocked")
        .unwrap();
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
fn guarded_panics_are_converted_and_unrelated_panics_reach_hooks_in_either_order() {
    match std::env::var(PANIC_HOOK_CHILD_ENV).as_deref() {
        Ok(order @ ("before" | "after")) => run_panic_hook_child(order),
        _ => {
            for order in ["before", "after"] {
                let status = Command::new(std::env::current_exe().unwrap())
                    .arg("--exact")
                    .arg(
                        "guarded_panics_are_converted_and_unrelated_panics_reach_hooks_in_either_order",
                    )
                    .arg("--nocapture")
                    .env(PANIC_HOOK_CHILD_ENV, order)
                    .status()
                    .unwrap();
                assert!(
                    status.success(),
                    "panic-hook child failed for {order} order"
                );
            }
        }
    }
}

fn run_panic_hook_child(order: &str) {
    let hook_calls = Arc::new(AtomicUsize::new(0));
    if order == "after" {
        run_guarded(
            "install Cozo hook",
            ScriptMutability::Immutable,
            &CozoGuardConfig::default(),
            || Ok(()),
        )
        .unwrap();
    }
    let delegate = panic::take_hook();
    let hook_calls_for_hook = Arc::clone(&hook_calls);
    panic::set_hook(Box::new(move |info| {
        hook_calls_for_hook.fetch_add(1, Ordering::SeqCst);
        delegate(info);
    }));

    let successor_observes_guarded = usize::from(order == "after");
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
    assert_eq!(
        hook_calls.load(Ordering::SeqCst),
        successor_observes_guarded
    );

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
    assert_eq!(
        hook_calls.load(Ordering::SeqCst),
        successor_observes_guarded + 1
    );

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
    assert_eq!(
        hook_calls.load(Ordering::SeqCst),
        successor_observes_guarded * 3 + 1
    );
}
