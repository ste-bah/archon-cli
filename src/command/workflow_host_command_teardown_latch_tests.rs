//! The latch runs the post-teardown step exactly once, after the last
//! tracked teardown, and says whether every one was confirmed.
use super::*;

fn recorder() -> (Arc<Mutex<Vec<bool>>>, impl Fn() -> AfterTeardown) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let held = seen.clone();
    (seen, move || {
        let held = held.clone();
        Box::new(move |confirmed| held.lock().unwrap().push(confirmed))
    })
}

#[test]
fn with_no_tree_pending_the_step_runs_at_once_and_confirmed() {
    let latch = TeardownLatch::default();
    let (seen, step) = recorder();
    latch.after_teardown(step());
    assert_eq!(*seen.lock().unwrap(), vec![true]);
    latch.track().settled(true);
    latch.after_teardown(step());
    assert_eq!(*seen.lock().unwrap(), vec![true, true]);
}

#[test]
fn the_step_waits_for_the_last_pending_teardown() {
    let latch = TeardownLatch::default();
    let (seen, step) = recorder();
    let first = latch.track();
    let second = latch.track();
    latch.after_teardown(step());
    assert!(seen.lock().unwrap().is_empty(), "deferred while trees run");
    first.settled(true);
    assert!(seen.lock().unwrap().is_empty(), "one tree still runs");
    assert!(latch.pending());
    second.settled(true);
    assert_eq!(*seen.lock().unwrap(), vec![true]);
    assert!(!latch.pending());
}

#[test]
fn a_dropped_or_stalled_teardown_reports_unconfirmed_and_stays_so() {
    let latch = TeardownLatch::default();
    let (seen, step) = recorder();
    let token = latch.track();
    latch.after_teardown(step());
    drop(token);
    assert_eq!(*seen.lock().unwrap(), vec![false], "dropped is unconfirmed");
    latch.track().settled(true);
    latch.after_teardown(step());
    assert_eq!(
        *seen.lock().unwrap(),
        vec![false, false],
        "an earlier stall can still leave survivors"
    );
}
