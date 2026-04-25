//! Integration tests for TC-TUI-EVENTLOOP-01, 02, 04, 05, 06.
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use archon_core::agent::{AgentEvent, TimestampedEvent};
use archon_tui::{AgentDispatcher, AgentRouter, EventLoopConfig, TurnOutcome, TurnRunner};
use tokio::sync::mpsc::unbounded_channel;
use tokio::time::sleep;

// --- Test helpers ---

#[derive(Clone)]
struct RecordingRunner {
    log: Arc<Mutex<Vec<String>>>,
    delay_ms: u64,
}

impl RecordingRunner {
    fn new(log: Arc<Mutex<Vec<String>>>, delay_ms: u64) -> Self {
        Self { log, delay_ms }
    }
}

impl TurnRunner for RecordingRunner {
    fn run_turn<'a>(
        &'a self,
        prompt: String,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        let log = self.log.clone();
        let delay = self.delay_ms;
        Box::pin(async move {
            sleep(Duration::from_millis(delay)).await;
            log.lock().unwrap().push(prompt);
            Ok(())
        })
    }
}

struct SlowRunner {
    delay_ms: u64,
}

impl SlowRunner {
    fn new(delay_ms: u64) -> Self {
        Self { delay_ms }
    }
}

impl TurnRunner for SlowRunner {
    fn run_turn<'a>(
        &'a self,
        _prompt: String,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        let delay = self.delay_ms;
        Box::pin(async move {
            sleep(Duration::from_millis(delay)).await;
            Ok(())
        })
    }
}

struct NoopRouter;
impl AgentRouter for NoopRouter {
    fn switch(&self, _agent_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Records every `switch` call (D2: local fake implementing AgentRouter trait).
struct RecordingRouter {
    calls: Arc<Mutex<Vec<String>>>,
}

impl RecordingRouter {
    fn new() -> (Self, Arc<Mutex<Vec<String>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }
}

impl AgentRouter for RecordingRouter {
    fn switch(&self, agent_id: &str) -> anyhow::Result<()> {
        self.calls.lock().unwrap().push(agent_id.to_string());
        Ok(())
    }
}

/// Drains the dispatcher to idle, collecting every TurnOutcome.
async fn drain_until_idle(d: &mut AgentDispatcher) -> Vec<TurnOutcome> {
    let mut outcomes = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while (d.is_busy() || d.queue_len() > 0) && Instant::now() < deadline {
        if let Some(outcome) = d.poll_completion() {
            outcomes.push(outcome);
        } else {
            sleep(Duration::from_millis(1)).await;
        }
    }
    while let Some(outcome) = d.poll_completion() {
        outcomes.push(outcome);
    }
    outcomes
}

// --- TC-01: spawn_turn returns without awaiting (REQ-TUI-LOOP-001) ---

/// Verifies that `spawn_turn` returns immediately without awaiting the agent
/// future. Uses a SlowRunner (5 s) and measures wall time inside spawn_turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_tc_01_input_handler_spawns_without_await() {
    let (agent_event_tx, _agent_event_rx) = unbounded_channel::<TimestampedEvent>();
    let router: Arc<dyn AgentRouter> = Arc::new(NoopRouter);
    let mut dispatcher = AgentDispatcher::new(router, agent_event_tx);

    let runner: Arc<dyn TurnRunner> = Arc::new(SlowRunner::new(5000));

    // Measure wall time around the spawn call — critical NFR.
    let start = Instant::now();
    let result = dispatcher.spawn_turn("long".into(), runner);
    let elapsed = start.elapsed();

    assert!(
        matches!(result, archon_tui::DispatchResult::Running { .. }),
        "spawn_turn should return Running"
    );
    assert!(
        elapsed < Duration::from_millis(50),
        "spawn_turn blocked for {} ms",
        elapsed.as_millis()
    );
    assert!(
        dispatcher.is_busy(),
        "dispatcher should be busy immediately after spawn_turn"
    );
    assert!(
        dispatcher.current_handle_is_inflight(),
        "current_handle_is_inflight should be true after spawn"
    );

    let _ = dispatcher.cancel_current(); // cleanup
}

// --- TC-02: current_query tracking during turn and after cancel (REQ-TUI-LOOP-002) ---

/// Verifies `current_handle_is_inflight()` tracks the in-flight handle during a
/// turn, returns false after natural completion, and false after cancel.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_tc_02_current_query_tracking() {
    let (agent_event_tx, _agent_event_rx) = unbounded_channel::<TimestampedEvent>();
    let router: Arc<dyn AgentRouter> = Arc::new(NoopRouter);
    let mut dispatcher = AgentDispatcher::new(router, agent_event_tx);

    let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let runner: Arc<dyn TurnRunner> = Arc::new(RecordingRunner::new(log.clone(), 50));

    // Phase 1: natural completion
    let _ = dispatcher.spawn_turn("first".into(), runner.clone());
    assert!(
        dispatcher.current_handle_is_inflight(),
        "handle should be in-flight just after spawn"
    );

    let outcomes = drain_until_idle(&mut dispatcher).await;
    assert_eq!(outcomes.len(), 1, "expected exactly one TurnOutcome");
    assert!(matches!(outcomes[0], TurnOutcome::Completed));
    assert!(
        !dispatcher.current_handle_is_inflight(),
        "handle should NOT be in-flight after completion"
    );
    assert!(
        !dispatcher.is_busy(),
        "dispatcher should not be busy after completion"
    );

    // Phase 2: cancel
    let slow_runner: Arc<dyn TurnRunner> = Arc::new(SlowRunner::new(10_000));
    let _ = dispatcher.spawn_turn("second".into(), slow_runner);
    assert!(
        dispatcher.current_handle_is_inflight(),
        "handle should be in-flight before cancel"
    );

    let cancel_result = dispatcher.cancel_current();
    assert!(
        matches!(cancel_result, archon_tui::CancelOutcome::Aborted { .. }),
        "cancel should return Aborted"
    );

    tokio::task::yield_now().await; // Guardrail D1: let abort land before checking state
    assert!(
        !dispatcher.current_handle_is_inflight(),
        "handle should NOT be in-flight after cancel"
    );
    assert!(
        !dispatcher.is_busy(),
        "dispatcher should not be busy after cancel"
    );
}

// --- TC-04: Burst 10 messages FIFO no loss (REQ-TUI-LOOP-004 / EC-TUI-003) ---

#[derive(Clone)]
enum BurstOutcome {
    Success,
    SleepForever,
}

struct BurstRunner {
    outcomes: Arc<Mutex<VecDeque<BurstOutcome>>>,
    recorded: Arc<Mutex<Vec<String>>>,
    run_delay_ms: u64,
}

impl BurstRunner {
    fn new(
        outcomes: Vec<BurstOutcome>,
        recorded: Arc<Mutex<Vec<String>>>,
        run_delay_ms: u64,
    ) -> Self {
        Self {
            outcomes: Arc::new(Mutex::new(outcomes.into())),
            recorded,
            run_delay_ms,
        }
    }
}

impl TurnRunner for BurstRunner {
    fn run_turn<'a>(
        &'a self,
        prompt: String,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        let outcome = self
            .outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(BurstOutcome::Success);
        let recorded = Arc::clone(&self.recorded);
        let delay = self.run_delay_ms;
        Box::pin(async move {
            if delay > 0 {
                sleep(Duration::from_millis(delay)).await;
            }
            recorded.lock().unwrap().push(prompt.clone());
            match outcome {
                BurstOutcome::Success => Ok(()),
                BurstOutcome::SleepForever => {
                    sleep(Duration::from_secs(3600)).await;
                    Ok(())
                }
            }
        })
    }
}

/// Verifies 10 prompts queued during an in-flight turn drain FIFO with zero loss.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_tc_04_burst_10_messages_fifo_no_loss() {
    let (agent_event_tx, _agent_event_rx) = unbounded_channel::<TimestampedEvent>();
    let router: Arc<dyn AgentRouter> = Arc::new(NoopRouter);
    let mut dispatcher = AgentDispatcher::new(router, agent_event_tx);

    let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let outcomes: Vec<BurstOutcome> = std::iter::once(BurstOutcome::Success)
        .chain(std::iter::repeat(BurstOutcome::Success).take(10))
        .collect();
    let runner: Arc<dyn TurnRunner> =
        Arc::new(BurstRunner::new(outcomes, Arc::clone(&recorded), 200));

    let _ = dispatcher.spawn_turn("m0".into(), Arc::clone(&runner));
    for i in 1..=10 {
        let label = format!("m{}", i);
        let q = dispatcher.spawn_turn(label.clone(), Arc::clone(&runner));
        assert!(
            matches!(q, archon_tui::DispatchResult::Queued),
            "prompt {} should be queued",
            label
        );
    }
    assert_eq!(
        dispatcher.queue_len(),
        10,
        "exactly 10 prompts should be queued"
    );

    sleep(Duration::from_secs(3)).await;
    let deadline = Instant::now() + Duration::from_secs(5);
    while (dispatcher.is_busy() || dispatcher.queue_len() > 0) && Instant::now() < deadline {
        let _ = dispatcher.poll_completion();
        sleep(Duration::from_millis(1)).await;
    }

    let got = recorded.lock().unwrap().clone();
    let expected: Vec<String> = (0..=10).map(|i| format!("m{}", i)).collect();
    assert_eq!(got.len(), 11, "expected 11 recorded prompts (zero loss)");
    assert_eq!(got, expected, "FIFO order violated");
}

// --- TC-05: Agent switch mid-flight (REQ-TUI-LOOP-005 / EC-TUI-004) ---

/// Verifies `switch_agent` delegates to router's `switch` method and does NOT
/// touch the in-flight handle — which remains in-flight after switch.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_tc_05_agent_switch_mid_flight() {
    let (agent_event_tx, _agent_event_rx) = unbounded_channel::<TimestampedEvent>();
    let (router_fake, calls) = RecordingRouter::new();
    let router: Arc<dyn AgentRouter> = Arc::new(router_fake);
    let mut dispatcher = AgentDispatcher::new(router, agent_event_tx);

    let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sleeper: Arc<dyn TurnRunner> = Arc::new(BurstRunner::new(
        vec![BurstOutcome::SleepForever],
        recorded,
        0,
    ));
    let _ = dispatcher.spawn_turn("prompt-A".into(), sleeper);

    assert!(
        dispatcher.is_busy(),
        "dispatcher should be busy with prompt-A in-flight"
    );
    assert!(
        dispatcher.current_handle_is_inflight(),
        "handle should be in-flight before switch"
    );

    dispatcher
        .switch_agent("agent-B")
        .expect("switch_agent should succeed");
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &["agent-B".to_string()],
        "router should have received agent-B"
    );
    assert!(
        dispatcher.current_handle_is_inflight(),
        "handle must still be in-flight after switch_agent"
    );
    assert!(
        dispatcher.is_busy(),
        "dispatcher should still be busy after switch"
    );

    let _ = dispatcher.cancel_current(); // cleanup
}

// --- TC-06: SIGWINCH reflow no frame drop (REQ-TUI-LOOP-006 / EC-TUI-005) ---

struct StreamRunner {
    frames: Arc<Mutex<Vec<usize>>>,
    frame_count: usize,
    interval_ms: u64,
}

impl StreamRunner {
    fn new(frames: Arc<Mutex<Vec<usize>>>, frame_count: usize, interval_ms: u64) -> Self {
        Self {
            frames,
            frame_count,
            interval_ms,
        }
    }
}

impl TurnRunner for StreamRunner {
    fn run_turn<'a>(
        &'a self,
        _prompt: String,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        let frames = Arc::clone(&self.frames);
        let count = self.frame_count;
        let interval = self.interval_ms;
        Box::pin(async move {
            sleep(Duration::from_millis(50)).await;
            for i in 0..count {
                frames.lock().unwrap().push(i + 1);
                sleep(Duration::from_millis(interval)).await;
            }
            Ok(())
        })
    }
}

/// Verifies Resize updates last_known_size and 20 stream frames are recorded.
///
/// TASK-200: `#[serial]` because this test both WRITES to the process-
/// global `LAST_KNOWN_SIZE` (via TuiEvent::Resize → run_event_loop →
/// handle_resize) and READS it back. Races against any other resize-
/// dispatching test under --test-threads=2. Coordinated with the
/// default-key `#[serial]` tests in src/layout_tests.rs and
/// event_loop_inner_coverage.rs within their respective binaries.
#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_tc_06_sigwinch_reflow_no_frame_drop() {
    let frames: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));
    let frames_clone = Arc::clone(&frames);

    let (tui_event_tx, tui_event_rx) = unbounded_channel::<archon_tui::app::TuiEvent>();
    let (agent_event_tx, _agent_event_rx) = unbounded_channel::<TimestampedEvent>();
    let runner: Arc<dyn TurnRunner> = Arc::new(StreamRunner::new(frames_clone, 20, 10));
    let router: Arc<dyn AgentRouter> = Arc::new(NoopRouter);

    let cfg = EventLoopConfig {
        tui_event_rx,
        agent_event_tx,
        runner,
        router,
    };
    let handle = tokio::spawn(async move { archon_tui::run_event_loop(cfg).await });

    tui_event_tx
        .send(archon_tui::app::TuiEvent::UserInput("stream".into()))
        .expect("send UserInput(stream)");
    tui_event_tx
        .send(archon_tui::app::TuiEvent::Resize {
            cols: 200,
            rows: 60,
        })
        .expect("send first Resize");
    tui_event_tx
        .send(archon_tui::app::TuiEvent::Resize {
            cols: 200,
            rows: 60,
        })
        .expect("send second Resize");

    sleep(Duration::from_millis(500)).await;
    tui_event_tx
        .send(archon_tui::app::TuiEvent::Done)
        .expect("send Done");
    handle
        .await
        .expect("join run_event_loop")
        .expect("run_event_loop Ok");

    let (cols, rows) = archon_tui::last_known_size();
    assert_eq!(
        (cols, rows),
        (200, 60),
        "last_known_size() should be (200, 60)"
    );

    let recorded = frames.lock().unwrap().clone();
    assert_eq!(recorded.len(), 20, "expected exactly 20 frames recorded");
    let expected: Vec<usize> = (1..=20).collect();
    assert_eq!(recorded, expected, "frames should be 1..=20 in order");
}
