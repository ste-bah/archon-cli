//! Contention retries stop after one no-progress window, as a typed pause.
//!
//! These use the production interactive settings: its window is the shortest
//! one a real caller gets. A retry loop with no progress check never returns,
//! so each case runs on its own thread and gives up after `GIVE_UP`.
use std::sync::mpsc;
use std::time::Instant;

use super::*;

const GIVE_UP: Duration = Duration::from_secs(30);

fn interactive(temp: &tempfile::TempDir) -> (CozoGuardConfig, PathBuf) {
    let db_path = temp.path().join("interactive.db");
    let config = CozoGuardConfig::for_interactive_db_path(&db_path);
    let lock = config.write_lock_path.clone().unwrap();
    (config, lock)
}

fn on_thread<T: Send + 'static>(
    run: impl FnOnce() -> T + Send + 'static,
) -> mpsc::Receiver<(T, Instant)> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let value = run();
        let _ = sender.send((value, Instant::now()));
    });
    receiver
}

fn assert_pause(error: &anyhow::Error, names: &str) {
    let busy = StoreBusy::find(error.as_ref())
        .unwrap_or_else(|| panic!("the pause must stay typed StoreBusy: {error:#}"));
    assert!(
        busy.detail.contains(names),
        "the pause must name {names}: {busy}"
    );
    assert!(busy.attempts > 1, "a pause follows real retries: {busy}");
}

#[test]
fn raw_busy_with_no_writer_progress_pauses_after_the_interactive_window() {
    let temp = tempfile::tempdir().unwrap();
    let (config, lock) = interactive(&temp);
    let wait = config.busy_wait;
    let started = Instant::now();
    let (result, returned) = on_thread(move || {
        run_guarded("stalled read", ScriptMutability::Immutable, &config, || {
            Err::<(), _>(anyhow!("list chunks failed: database is locked (code 5)"))
        })
    })
    .recv_timeout(GIVE_UP)
    .expect("raw busy retried with no progress limit");
    let error = result.unwrap_err();
    assert_pause(&error, &lock.display().to_string());
    assert!(format!("{error:#}").contains("database is locked (code 5)"));
    assert!(returned.duration_since(started) >= wait);
}

#[test]
fn fail_fast_lock_with_a_wedged_holder_pauses_after_the_interactive_window() {
    let temp = tempfile::tempdir().unwrap();
    let (config, lock) = interactive(&temp);
    let mut held = fd_lock::RwLock::new(std::fs::File::create(&lock).unwrap());
    let _guard = held.try_write().unwrap();
    let (result, _) = on_thread(move || {
        run_guarded("stalled write", ScriptMutability::Mutable, &config, || {
            Ok(())
        })
    })
    .recv_timeout(GIVE_UP)
    .expect("fail-fast lock contention retried with no progress limit");
    let error = result.unwrap_err();
    assert_pause(&error, &lock.display().to_string());
    assert!(format!("{error:#}").contains("write lock unavailable"));
}

#[tokio::test]
async fn async_raw_busy_with_no_writer_progress_pauses_after_the_interactive_window() {
    let temp = tempfile::tempdir().unwrap();
    let (config, lock) = interactive(&temp);
    let result = tokio::time::timeout(
        GIVE_UP,
        run_guarded_async(
            "stalled async read",
            ScriptMutability::Immutable,
            &config,
            || Err::<(), _>(anyhow!("database is locked (code 5)")),
        ),
    )
    .await
    .expect("async raw busy retried with no progress limit");
    assert_pause(&result.unwrap_err(), &lock.display().to_string());
}

#[test]
fn raw_busy_without_a_lock_path_pauses_and_says_so() {
    let temp = tempfile::tempdir().unwrap();
    let (config, _) = interactive(&temp);
    let config = CozoGuardConfig {
        write_lock_path: None,
        ..config
    };
    let (result, _) = on_thread(move || {
        run_guarded(
            "unpathed read",
            ScriptMutability::Immutable,
            &config,
            || Err::<(), _>(anyhow!("database is locked (code 5)")),
        )
    })
    .recv_timeout(GIVE_UP)
    .expect("unpathed raw busy retried with no progress limit");
    assert_pause(&result.unwrap_err(), "no write lock path");
}

#[test]
fn raw_busy_keeps_retrying_past_the_window_while_a_writer_commits() {
    let temp = tempfile::tempdir().unwrap();
    let (config, _) = interactive(&temp);
    let db_path = temp.path().join("interactive.db");
    let db = DbInstance::new("sqlite", db_path.to_str().unwrap(), "").unwrap();
    db.run_script(
        ":create progress { key: Int => value: Int }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    let busy_for = config.busy_wait + Duration::from_secs(2);
    let (stop, stopped) = mpsc::channel::<()>();
    let writer_config = config.clone();
    let writer = std::thread::spawn(move || {
        let mut value = 0;
        while stopped.recv_timeout(Duration::from_millis(200)).is_err() {
            value += 1;
            run_script_guarded(
                &db,
                &format!("?[key, value] <- [[1, {value}]] :put progress {{key => value}}"),
                Default::default(),
                ScriptMutability::Mutable,
                "committing writer",
                &writer_config,
            )
            .unwrap();
        }
    });
    let started = Instant::now();
    let result = run_guarded("reader", ScriptMutability::Immutable, &config, || {
        if started.elapsed() < busy_for {
            Err(anyhow!("database is locked (code 5)"))
        } else {
            Ok(42)
        }
    });
    stop.send(()).unwrap();
    writer.join().unwrap();
    assert_eq!(
        result.expect("committed writes must renew the no-progress window"),
        42
    );
}
