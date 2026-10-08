use super::*;

fn config() -> CozoGuardConfig {
    CozoGuardConfig {
        max_attempts: 2,
        initial_backoff: Duration::ZERO,
        max_backoff: Duration::ZERO,
        ..Default::default()
    }
}
fn assert_busy_attempts(error: anyhow::Error, attempts: usize) {
    let busy = error
        .downcast_ref::<StoreBusy>()
        .expect("exhaustion must be explicitly retryable busy");
    assert_eq!(busy.attempts, attempts);
    assert!(busy.to_string().contains("operation not completed"));
    assert!(is_retryable_cozo_error(&busy.to_string()));
}

fn assert_busy(error: anyhow::Error) {
    assert_busy_attempts(error, 2);
}

#[test]
fn paused_read_returns_explicit_busy_then_can_be_retried() {
    let config = config();
    assert_busy(
        run_guarded("read", ScriptMutability::Immutable, &config, || {
            Err::<usize, _>(
                StoreBusy {
                    context: "read".into(),
                    attempts: 2,
                    detail: "acquisition paused".into(),
                }
                .into(),
            )
        })
        .unwrap_err(),
    );
    assert_eq!(
        run_guarded("read", ScriptMutability::Immutable, &config, || Ok(42)).unwrap(),
        42
    );
}
#[tokio::test]
async fn paused_async_read_returns_explicit_busy_then_can_be_retried() {
    let config = config();
    assert_busy(
        run_guarded_async("async read", ScriptMutability::Immutable, &config, || {
            Err::<usize, _>(
                StoreBusy {
                    context: "read".into(),
                    attempts: 2,
                    detail: "acquisition paused".into(),
                }
                .into(),
            )
        })
        .await
        .unwrap_err(),
    );
    assert_eq!(
        run_guarded_async("async read", ScriptMutability::Immutable, &config, || Ok(
            42
        ))
        .await
        .unwrap(),
        42
    );
}
#[test]
fn paused_writer_returns_explicit_busy_then_can_be_retried() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("code500.lock");
    let lock = std::fs::File::create(&path).unwrap();
    let mut lock = fd_lock::RwLock::new(lock);
    let _held = lock.try_write().unwrap();
    let config = config()
        .with_write_lock_path(&path)
        .with_write_lock_wait(Duration::ZERO);
    assert_busy_attempts(
        run_guarded("writer", ScriptMutability::Mutable, &config, || Ok(42)).unwrap_err(),
        1,
    );
    drop(_held);
    assert_eq!(
        run_guarded("writer", ScriptMutability::Mutable, &config, || Ok(42)).unwrap(),
        42
    );
}

#[test]
fn file_lock_window_expiry_is_busy_and_does_not_run_the_body() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("code500.lock");
    let mut lock = fd_lock::RwLock::new(std::fs::File::create(&path).unwrap());
    let held = lock.try_write().unwrap();
    let ran = std::cell::Cell::new(false);
    let error =
        with_write_lock_blocking_timeout(&path, "file wait", Duration::from_millis(30), || {
            ran.set(true);
            Ok(42)
        })
        .unwrap_err();
    assert!(!ran.get());
    drop(held);
    assert_busy_attempts(error, 1);
    assert_eq!(
        with_write_lock_blocking(&path, "retry", || Ok(42)).unwrap(),
        42
    );
}

#[test]
fn process_lock_window_expiry_is_busy_and_does_not_run_the_body() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("code500.lock");
    let (ready, started) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let holder_path = path.clone();
    let holder = std::thread::spawn(move || {
        with_write_lock_blocking(&holder_path, "holder", || {
            ready.send(()).unwrap();
            released.recv().unwrap();
            Ok(())
        })
        .unwrap();
    });
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    let ran = std::cell::Cell::new(false);
    let error =
        with_write_lock_blocking_timeout(&path, "process wait", Duration::from_millis(30), || {
            ran.set(true);
            Ok(42)
        })
        .unwrap_err();
    release.send(()).unwrap();
    holder.join().unwrap();
    assert!(!ran.get());
    assert_busy_attempts(error, 1);
    assert_eq!(
        with_write_lock_blocking(&path, "retry", || Ok(42)).unwrap(),
        42
    );
}

#[test]
fn queued_guard_window_expiry_is_busy_without_restarting_acquisition() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("code500.lock");
    let mut lock = fd_lock::RwLock::new(std::fs::File::create(&path).unwrap());
    let held = lock.try_write().unwrap();
    let config = config()
        .with_write_lock_path(&path)
        .with_write_lock_wait(Duration::from_millis(30));
    let ran = std::cell::Cell::new(false);
    let error = run_guarded("queued wait", ScriptMutability::Mutable, &config, || {
        ran.set(true);
        Ok(42)
    })
    .unwrap_err();
    assert!(!ran.get());
    drop(held);
    assert_busy_attempts(error, 1);
    assert_eq!(
        run_guarded("retry", ScriptMutability::Mutable, &config, || Ok(42)).unwrap(),
        42
    );
}
