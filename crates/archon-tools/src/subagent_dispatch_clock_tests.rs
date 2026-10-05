use super::*;

fn secs(value: u64) -> Duration {
    Duration::from_secs(value)
}

#[tokio::test(start_paused = true)]
async fn the_clock_counts_execution_and_not_the_slot_wait() {
    let clock = DispatchClock::new();
    tokio::time::sleep(secs(10)).await;
    let pause = clock.pause_for_slot();
    assert!(clock.waiting_for_slot());
    tokio::time::sleep(secs(500)).await;
    assert_eq!(clock.elapsed(), secs(10), "the wait is not counted");
    drop(pause);
    tokio::time::sleep(secs(5)).await;
    assert_eq!(clock.elapsed(), secs(15));
}

#[tokio::test(start_paused = true)]
async fn a_deadline_never_fires_during_a_slot_wait() {
    let clock = DispatchClock::new();
    let pause = clock.pause_for_slot();
    let started = Instant::now();
    let waiter = {
        let clock = Arc::clone(&clock);
        tokio::spawn(async move { clock.exceeding(secs(100)).await })
    };
    tokio::time::sleep(secs(1_000)).await;
    assert!(!waiter.is_finished(), "a queued call is never cut");
    drop(pause);
    waiter.await.unwrap();
    assert_eq!(
        Instant::now() - started,
        secs(1_100),
        "the bound starts when the slot is acquired"
    );
}

#[tokio::test(start_paused = true)]
async fn within_excludes_slot_waits_reported_by_a_session() {
    let output = within(secs(100), async {
        let call = current_call().expect("the call clock is installed");
        scope_session("agent-a", vec![call], async {
            let pause = slot_wait("agent-a").expect("clocks for this session");
            tokio::time::sleep(secs(1_000)).await;
            drop(pause);
            tokio::time::sleep(secs(60)).await;
            "ran"
        })
        .await
    })
    .await;
    assert_eq!(output, Some("ran"));
}

#[tokio::test(start_paused = true)]
async fn within_still_cuts_execution_past_the_limit() {
    let started = Instant::now();
    let output = within(secs(100), tokio::time::sleep(secs(1_000))).await;
    assert_eq!(output, None);
    assert_eq!(Instant::now() - started, secs(100));
}

#[tokio::test]
async fn a_child_session_never_stops_its_parents_clocks() {
    let clock = DispatchClock::new();
    scope_session("parent", vec![Arc::clone(&clock)], async {
        assert!(slot_wait("child").is_none());
        assert!(!clock.waiting_for_slot());
        let pause = slot_wait("parent").expect("the parent's own wait");
        assert!(clock.waiting_for_slot());
        drop(pause);
        assert!(!clock.waiting_for_slot());
    })
    .await;
    assert!(slot_wait("parent").is_none(), "outside the scope");
}
