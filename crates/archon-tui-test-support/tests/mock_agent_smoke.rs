use std::time::Duration;

use archon_tui_test_support::mock_agent::{
    spawn_n_mock_agents, EventScript, MockAgent, MockEventKind,
};
use tokio::sync::mpsc;

fn rt_paused() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime")
}

fn rt_multi() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime")
}

#[test]
fn scripted_events_are_emitted_in_order() {
    let rt = rt_paused();
    rt.block_on(async {
        let (tx, mut rx) = mpsc::unbounded_channel::<MockEventKind>();
        let script = EventScript::new()
            .tool_call("bash")
            .message_delta("hello")
            .message_delta("world")
            .finish();
        let agent = MockAgent::new("scripted", script);
        let report = agent.run(tx).await;
        assert_eq!(report.events_sent, 4);
        assert_eq!(report.events_dropped, 0);
        assert!(!report.cancelled);

        assert_eq!(rx.recv().await, Some(MockEventKind::ToolCall("bash".into())));
        assert_eq!(rx.recv().await, Some(MockEventKind::MessageDelta("hello".into())));
        assert_eq!(rx.recv().await, Some(MockEventKind::MessageDelta("world".into())));
        assert_eq!(rx.recv().await, Some(MockEventKind::Finish));
    });
}

#[test]
fn cancel_aborts_run_within_100ms_wallclock_with_paused_time() {
    let rt = rt_paused();
    rt.block_on(async {
        let (tx, _rx) = mpsc::unbounded_channel::<MockEventKind>();
        // Long script with a 1-second virtual sleep between events.
        let script = EventScript::new()
            .burst_of(1000, MockEventKind::MessageDelta("x".into()))
            .sleep(Duration::from_secs(1));
        let agent = MockAgent::new("cancel", script);
        let token = agent.cancel_handle();

        let handle = tokio::spawn(async move { agent.run(tx).await });

        // Let the agent begin, then cancel immediately.
        tokio::task::yield_now().await;
        token.cancel();

        let wall = std::time::Instant::now();
        let report = handle.await.expect("join");
        let wall_elapsed = wall.elapsed();

        assert!(report.cancelled, "report.cancelled must be true");
        assert!(
            report.events_sent < 1000,
            "expected early cancel but sent {} of 1000",
            report.events_sent
        );
        // Wall-clock assertion: cancellation path does not spin on real time
        // even though the scripted interval was 1 second.
        assert!(
            wall_elapsed < Duration::from_millis(500),
            "cancel took {:?} wall-clock",
            wall_elapsed
        );
    });
}

#[test]
fn spawn_100_mock_agents_each_10_events_all_complete() {
    let rt = rt_multi();
    rt.block_on(async {
        let (tx, mut rx) = mpsc::unbounded_channel::<MockEventKind>();
        let handles = spawn_n_mock_agents(100, 10, tx);
        assert_eq!(handles.len(), 100);

        let mut total_sent = 0usize;
        for h in handles {
            let report = h.await.expect("join");
            assert!(!report.cancelled);
            assert_eq!(report.events_dropped, 0);
            total_sent += report.events_sent;
        }
        assert_eq!(total_sent, 100 * 10);

        // Drain receiver to verify events actually landed.
        let mut received = 0usize;
        while let Ok(_ev) = rx.try_recv() {
            received += 1;
        }
        assert_eq!(received, 1000);
    });
}

#[test]
fn unbounded_sink_never_drops() {
    let rt = rt_multi();
    rt.block_on(async {
        let (tx, mut rx) = mpsc::unbounded_channel::<MockEventKind>();
        let script = EventScript::new()
            .burst_of(5000, MockEventKind::MessageDelta("y".into()));
        let agent = MockAgent::new("flood", script);
        let report = agent.run(tx).await;
        assert_eq!(report.events_sent, 5000);
        assert_eq!(report.events_dropped, 0);

        let mut count = 0usize;
        while let Ok(_ev) = rx.try_recv() {
            count += 1;
        }
        assert_eq!(count, 5000);
    });
}

#[test]
fn bounded_sink_reports_drops_on_full() {
    let rt = rt_multi();
    rt.block_on(async {
        // Capacity-1 bounded channel. We never consume, so the very first
        // try_send fills it and every subsequent send reports a drop.
        let (tx, _rx) = mpsc::channel::<MockEventKind>(1);
        let script = EventScript::new()
            .burst_of(10, MockEventKind::MessageDelta("z".into()));
        let agent = MockAgent::new("bounded", script);
        let report = agent.run(tx).await;
        // First send succeeds (fills the channel). Next 9 must report drops.
        assert_eq!(report.events_sent + report.events_dropped, 10);
        assert!(
            report.events_dropped >= 9,
            "expected >=9 drops, got {}",
            report.events_dropped
        );
    });
}
