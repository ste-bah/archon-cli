use super::*;
use futures::FutureExt;

// The callback renews just before the OLD deadline, then holds evaluation
// until that deadline passes. The NEW deadline is still far in the future.
fn interleaved_renewal() -> Arc<NoProgressWindow> {
    let window = NoProgressWindow::new(Duration::from_secs(1));
    window.progress.send_modify(|state| {
        state.last_output = Instant::now() - Duration::from_millis(900);
    });
    let renewed = Arc::clone(&window);
    BEFORE_CHECK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            renewed.record_output();
            // advance updates the paused clock before yielding. Poll once so
            // the synchronous expiry seam can cross the old deadline without
            // giving the waiter an opportunity to take a new snapshot.
            let before = Instant::now();
            let _ = tokio::time::advance(Duration::from_millis(150)).now_or_never();
            assert_eq!(Instant::now() - before, Duration::from_millis(150));
        }));
    });
    window
}

#[tokio::test(start_paused = true)]
async fn renewal_between_snapshot_and_expired_evaluation_is_observed() {
    assert!(!interleaved_renewal().expired());
}

#[tokio::test(start_paused = true)]
async fn renewal_between_snapshot_and_wait_evaluation_is_observed() {
    let result = interleaved_renewal()
        .wait("test", std::future::ready(42))
        .await;
    assert!(matches!(result, Ok(42)), "{result:?}");
}

#[tokio::test(start_paused = true)]
async fn expiry_checks_latch_the_stop_under_the_state_lock() {
    let window = NoProgressWindow::new(Duration::from_secs(1));
    window
        .progress
        .send_modify(|state| state.last_output = Instant::now() - Duration::from_secs(2));
    assert!(window.expired());
    assert!(window.progress.borrow().expired, "expiry must be latched");
}
