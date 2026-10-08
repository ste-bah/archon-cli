//! Real successful mutations reset acquisition's inactivity window.
use archon_cozo::{CozoGuardConfig, StoreBusy, run_guarded, with_write_lock_blocking_timeout};
use cozo::{DbInstance, ScriptMutability};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

fn progressing_holder(process_mutex: bool, stalls: bool) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("progress.db");
    let lock_path = archon_cozo::write_lock_path_for_db(&path);
    let db = Arc::new(DbInstance::new("sqlite", path.to_str().unwrap(), "").unwrap());
    db.run_script(
        ":create progress { key: Int => value: Int }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    let (ready, started) = mpsc::channel();
    let (last, written) = mpsc::channel();
    let writer_lock = lock_path.clone();
    let window = Duration::from_millis(700);
    let holder = std::thread::spawn(move || {
        let mutate = || {
            ready.send(()).unwrap();
            let writes = if stalls { 4 } else { 14 };
            for value in 0..writes {
                db.run_script(
                    &format!("?[key, value] <- [[1, {value}]] :put progress {{key => value}}"),
                    Default::default(),
                    ScriptMutability::Mutable,
                )
                .unwrap();
                std::thread::sleep(Duration::from_millis(100));
            }
            last.send(Instant::now()).unwrap();
            if stalls {
                std::thread::sleep(window * 2);
            }
            Ok(())
        };
        if process_mutex {
            run_guarded(
                "progressing holder",
                ScriptMutability::Mutable,
                &CozoGuardConfig::default().with_write_lock_path(writer_lock),
                mutate,
            )
            .unwrap();
        } else {
            let mut lock = fd_lock::RwLock::new(std::fs::File::create(writer_lock).unwrap());
            let _held = lock.try_write().unwrap();
            mutate().unwrap();
        }
    });
    started.recv_timeout(Duration::from_secs(10)).unwrap();
    let result = with_write_lock_blocking_timeout(&lock_path, "waiting reader", window, || Ok(42));
    let returned = Instant::now();
    holder.join().unwrap();
    let last_write = written.recv().unwrap() - Duration::from_millis(100);
    if stalls {
        assert!(result.unwrap_err().is::<StoreBusy>(), "a stall is a pause");
        assert!(
            returned.duration_since(last_write) >= window,
            "acquisition expired on total lifetime, before a full no-progress window"
        );
    } else {
        assert_eq!(
            result.unwrap(),
            42,
            "successful writer mutations must reset the window"
        );
    }
}
#[test]
fn process_acquisition_waits_while_real_mutations_progress() {
    progressing_holder(true, false);
}
#[test]
fn file_acquisition_waits_while_real_mutations_progress() {
    progressing_holder(false, false);
}
#[test]
fn acquisition_pauses_only_after_the_last_real_mutation_stalls() {
    progressing_holder(false, true);
}
