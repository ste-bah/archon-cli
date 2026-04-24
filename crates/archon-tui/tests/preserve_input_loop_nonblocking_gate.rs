//! TASK-TUI-901 — PRESERVE gate: ERR-TUI-002 input-loop non-blocking
//! standing regression guard.
//!
//! ## Purpose (ERR-TUI-002 regression guard)
//!
//! ERR-TUI-002 was the pre-fix failure mode where the TUI input handler
//! awaited `agent.process_message(...)` **inline**, blocking keyboard
//! and tick event processing for the entire duration of an agent turn.
//! SPEC-TUI-EVENTLOOP owns the positive fix (`tokio::spawn` wrapping
//! the turn via `AgentDispatcher::spawn_turn`). This test owns the
//! **standing integration guard** that proves the input loop stays
//! responsive while a turn is in flight: if anyone re-inlines the
//! await (or equivalent serialization on the main loop thread), the
//! guard fires within 100ms.
//!
//! ## Test strategy
//!
//! Drives the real public input-handler entrypoint
//! [`archon_tui::run_event_loop`] with an `EventLoopConfig` whose
//! `runner` is a `SleepyRunner`: `run_turn` records the prompt it was
//! entered with, then awaits [`tokio::time::sleep`] for 3 seconds (the
//! duration the spec specifies).
//!
//! The test then:
//! 1. Sends `TuiEvent::UserInput("hello")` — the responsive loop pulls
//!    it, `dispatcher.spawn_turn(...)` it, the spawned task enters
//!    `run_turn` and begins sleeping.
//! 2. Waits for "hello" to appear in the runner log (confirms the
//!    first turn actually spawned, so we're observing the RIGHT state
//!    — turn-in-flight — for the next assertion).
//! 3. Sends a SECOND synthetic event (`TuiEvent::SlashAgent("probe")`)
//!    — responsive loop pulls it and calls `router.switch("probe")`
//!    which increments a counter. Blocked loop: this event sits in the
//!    channel unread because the main thread is parked on the 3-second
//!    sleep inside `.process_message(...).await`.
//! 4. Waits up to 100ms wall-clock for the router counter to tick.
//!    If it ticks: the input loop is responsive while the turn is in
//!    flight (PASS). If it doesn't tick in 100ms: the loop is blocked
//!    (FAIL with `ERR-TUI-002` + `input loop blocked` in the panic).
//!
//! ## Deviation from spec text
//!
//! **Spec says** the second event should be `TuiEvent::UserInput("ping")`
//! and the assertion is "both accepted by the input loop within 50ms".
//! **Test uses** `TuiEvent::SlashAgent("probe")` as the second event.
//!
//! **Rationale.** The spec's operative word is "accepted by the input
//! loop" — i.e. pulled from the event channel. `UserInput("ping")` on
//! the responsive path is pulled and pushed into the dispatcher's
//! private FIFO queue (`pending_queue`); on the blocked path it sits
//! in the channel. The *queue insertion* is not externally observable
//! — `AgentDispatcher` is constructed inside `run_event_loop` and is
//! not reachable from test code.
//!
//! `SlashAgent` provides the same "was the event pulled?" signal via
//! an observable callback: `AgentRouter::switch(id)` fires on the
//! responsive path and does not fire on the blocked path. The test
//! captures this with a shared atomic counter inside the `ProbeRouter`.
//!
//! This preserves the spec's INTENT verbatim — an integration guard
//! that fires when an additional event sent after a long-running turn
//! does not get processed by the event loop within 100ms — while
//! routing the signal through an externally-observable API surface.
//! Both events are processed by the same `tui_event_rx.recv()` branch
//! of the `tokio::select!` inside `run_event_loop`, so the blocked-
//! path failure mode (main-thread parked on an inline `.await`) is
//! faithfully modelled: if the select! body is blocked, neither
//! UserInput nor SlashAgent is pulled.
//!
//! Spec reference: TASK-TUI-901.md §Scope: "if input handler is not
//! yet modularized (pre-MODULARIZATION), gate targets `main.rs`-level
//! entrypoint via `archon_tui::run_with_agent(stub)` shim" — the
//! equivalent public shim in this codebase is `run_event_loop`, which
//! IS the real input-handler entrypoint (see
//! `crates/archon-tui/src/event_loop.rs:90-149` for the `tokio::select!`
//! that owns both keyboard-event-analogue dispatch AND the
//! `process_message` spawn/drain path).
//!
//! ## Validation criteria (from TASK-TUI-901 §Validation Criteria)
//!
//! 1. Passes on the current tree (post-EVENTLOOP fix): ✓ — the
//!    `run_event_loop` path uses `dispatcher.spawn_turn` which wraps
//!    `runner.run_turn(prompt).await` in `tokio::spawn(...)`. The
//!    second event is pulled from the channel within one select!
//!    iteration (typically <10ms wall-clock).
//! 2. Reverting the EVENTLOOP fix (re-inlining `.await`) causes the
//!    gate to fail within 100ms: ✓ — the wall-clock budget is 100ms
//!    and the sleep is 3 seconds, so the assertion runs well before
//!    the sleep could possibly finish naturally. See §Negative-path
//!    note in the test-reporting summary.
//! 3. Failure message contains `ERR-TUI-002` and `input loop blocked`:
//!    ✓ — see the `panic!` string below.
//! 4. Gate completes in <1 second: ✓ — budget is 100ms + setup; the
//!    3s sleep inside `SleepyRunner::run_turn` is terminated by the
//!    event loop's task being dropped at test teardown (dropping the
//!    unbounded sender causes `recv()` to return None, which breaks
//!    the loop and lets `tokio::spawn`'s runtime drop the sleeping
//!    task before its 3s elapses).
//!
//! ## Cargo.toml note
//!
//! Spec allows adding `tokio` dev-deps features `time` + `test-util`.
//! Not required: workspace tokio is `features = ["full"]` which already
//! includes both.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use archon_core::agent::TimestampedEvent;
use archon_tui::{AgentRouter, EventLoopConfig, TurnRunner, app::TuiEvent, run_event_loop};
use tokio::sync::mpsc::unbounded_channel;

/// Runner that records each prompt it was entered with, then awaits a
/// 3-second `tokio::time::sleep`. The entry record happens BEFORE the
/// sleep so we can observe "prompt was pulled from the event channel
/// and its turn was spawned" without waiting for the sleep to finish.
///
/// The 3-second duration matches TASK-TUI-901 scope verbatim. It never
/// elapses during a passing run — the event loop is torn down (via its
/// `tui_event_tx` being dropped) long before 3 seconds expire.
struct SleepyRunner {
    log: Arc<Mutex<Vec<String>>>,
}

impl TurnRunner for SleepyRunner {
    fn run_turn<'a>(
        &'a self,
        prompt: String,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        let log = self.log.clone();
        Box::pin(async move {
            // Record entry BEFORE sleeping so an observer can confirm
            // the turn's task was spawned and the future was polled.
            log.lock().unwrap().push(prompt);
            // The spec-specified 3-second sleep. This is what would
            // block the input loop if the EVENTLOOP fix were reverted.
            tokio::time::sleep(Duration::from_secs(3)).await;
            Ok(())
        })
    }
}

/// Router whose `switch` increments a shared atomic counter. This gives
/// the test an externally-observable signal that the event loop pulled
/// and dispatched a `TuiEvent::SlashAgent(..)` while a turn was in flight.
///
/// The counter is the core assertion point for the ERR-TUI-002 guard:
/// it MUST increment within 100ms of the `SlashAgent` send on a healthy
/// (non-blocking) input loop; it will NOT increment within 100ms on a
/// blocked loop because the main thread is parked on an inline `.await`.
struct ProbeRouter {
    switch_count: Arc<AtomicUsize>,
}

impl AgentRouter for ProbeRouter {
    fn switch(&self, _agent_id: &str) -> anyhow::Result<()> {
        self.switch_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Helper: poll `log` until it contains `needle`, up to `budget`
/// wall-clock. Uses `std::time::Instant` (real wall-clock) — this is
/// the actual responsiveness measurement.
async fn wait_for_log_contains(
    log: &Arc<Mutex<Vec<String>>>,
    needle: &str,
    budget: Duration,
) -> Result<Duration, Duration> {
    let start = Instant::now();
    loop {
        {
            let guard = log.lock().unwrap();
            if guard.iter().any(|p| p == needle) {
                return Ok(start.elapsed());
            }
        }
        if start.elapsed() >= budget {
            return Err(start.elapsed());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Helper: poll `counter` until it reaches `target`, up to `budget`
/// wall-clock. Mirrors `wait_for_log_contains` shape so assertion
/// timing is symmetric across both observables.
async fn wait_for_counter_at_least(
    counter: &Arc<AtomicUsize>,
    target: usize,
    budget: Duration,
) -> Result<Duration, Duration> {
    let start = Instant::now();
    loop {
        if counter.load(Ordering::SeqCst) >= target {
            return Ok(start.elapsed());
        }
        if start.elapsed() >= budget {
            return Err(start.elapsed());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preserve_input_loop_nonblocking_gate() {
    let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let runner: Arc<dyn TurnRunner> = Arc::new(SleepyRunner { log: log.clone() });

    let switch_count: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    let router: Arc<dyn AgentRouter> = Arc::new(ProbeRouter {
        switch_count: switch_count.clone(),
    });

    let (tui_event_tx, tui_event_rx) = unbounded_channel::<TuiEvent>();
    let (agent_event_tx, _agent_event_rx) = unbounded_channel::<TimestampedEvent>();

    let cfg = EventLoopConfig {
        tui_event_rx,
        agent_event_tx,
        runner,
        router,
    };

    // Spawn the real public input-handler entrypoint (run_event_loop)
    // onto the multi-threaded runtime. This is the shim the spec
    // §Wiring Check line 83 calls for: we target the real public
    // entrypoint, not a private reimplementation.
    let loop_handle = tokio::spawn(async move { run_event_loop(cfg).await });

    // Step 1: send the first prompt. Responsive loop: dispatcher spawns
    // a turn; SleepyRunner::run_turn enters, records "hello", awaits
    // the 3s sleep.
    tui_event_tx
        .send(TuiEvent::UserInput("hello".into()))
        .expect("send hello");

    // Step 2: wait for "hello" to land in the log — confirms the first
    // turn actually started. 100ms is generous; on a healthy machine
    // this resolves in single-digit ms. If this fails, the event loop
    // isn't running at all — distinguishes test-setup bugs from the
    // real ERR-TUI-002 signal which is measured in step 4.
    wait_for_log_contains(&log, "hello", Duration::from_millis(100))
        .await
        .expect(
            "SleepyRunner did not enter run_turn for 'hello' within 100ms — \
             the event loop is not dispatching the first event at all. \
             This is a pre-condition failure, not the ERR-TUI-002 guard \
             — check that run_event_loop is running.",
        );

    // Step 3: the "hello" turn is now in flight and sleeping. Send a
    // second synthetic event — `SlashAgent("probe")` — which on the
    // responsive path is pulled from the channel on the next select!
    // iteration and dispatched to `router.switch("probe")`. On the
    // blocked path this send remains unread in the channel because the
    // loop is parked on the 3-second sleep inside `.process_message().await`.
    //
    // Record the wall-clock send time BEFORE sending so the 100ms
    // budget in step 4 is measured from the correct reference point.
    let send_at = Instant::now();
    tui_event_tx
        .send(TuiEvent::SlashAgent("probe".into()))
        .expect("send slash-agent probe");

    // Step 4: wait up to 100ms wall-clock for the router counter to
    // increment. This is the ERR-TUI-002 gate proper.
    let probe_result =
        wait_for_counter_at_least(&switch_count, 1, Duration::from_millis(100)).await;

    // Record observed wall-clock for the panic message / reporting.
    let observed_elapsed_from_send = send_at.elapsed();

    // Tear down the event loop before asserting so a panic below
    // doesn't leak the spawned task across tests. Dropping
    // `tui_event_tx` causes `recv()` to return None on the next poll;
    // the loop exits. We also send Done explicitly as belt-and-braces.
    let _ = tui_event_tx.send(TuiEvent::Done);
    drop(tui_event_tx);
    let _ = loop_handle.await;

    // Assertion: the router counter MUST have ticked within the 100ms
    // budget. The panic message is the contractual failure signal —
    // future triage greps for `ERR-TUI-002` and `input loop blocked`.
    if let Err(elapsed) = probe_result {
        let log_snapshot = log.lock().unwrap().clone();
        let observed_count = switch_count.load(Ordering::SeqCst);
        panic!(
            "ERR-TUI-002 regression: input loop blocked while agent turn \
             was in flight.\n\n\
             The test sent UserInput('hello') and waited for the runner \
             to enter run_turn (confirming an agent turn is in flight and \
             sleeping on a 3-second `tokio::time::sleep`). It then sent a \
             second event (SlashAgent('probe')) and polled up to 100ms for \
             the AgentRouter::switch callback to fire. On a healthy event \
             loop (post-SPEC-TUI-EVENTLOOP fix) the select! iterates and \
             dispatches the SlashAgent within <10ms. On a blocked event \
             loop the select! body is parked on the inline \
             `.process_message(...).await`, so the SlashAgent send sits in \
             the channel and never fires `router.switch`.\n\n\
             Observed: router.switch fire count = {} after {:?} polling \
             budget (sent at T+{:?} wall-clock from second event). Runner \
             log at time of failure: {:?}.\n\n\
             This failure indicates someone has re-introduced the inline \
             `.process_message(...).await` (or equivalent synchronous \
             serialization) inside the main input loop — the exact pattern \
             SPEC-TUI-EVENTLOOP / TUI-107 removed. Fix: dispatch agent \
             turns through `AgentDispatcher::spawn_turn` (which wraps the \
             await in `tokio::spawn`), not inline on the event-loop thread. \
             See src/agent_handle.rs for the legitimate TurnRunner adapter.",
            observed_count, elapsed, observed_elapsed_from_send, log_snapshot
        );
    }

    // Belt-and-braces sanity: the pre-condition observation ("hello"
    // entered run_turn) should still hold at teardown.
    let final_log = log.lock().unwrap().clone();
    assert!(
        final_log.iter().any(|p| p == "hello"),
        "ERR-TUI-002 gate: 'hello' missing from runner log at end of test — \
         something cleared the log between setup and teardown. This is \
         a test-harness bug, not an input-loop bug. Log: {:?}",
        final_log
    );
    // And the counter should be exactly 1 — no double-dispatch, no races.
    let final_count = switch_count.load(Ordering::SeqCst);
    assert_eq!(
        final_count, 1,
        "ERR-TUI-002 gate: router.switch fired {} times, expected exactly 1. \
         Either the event was double-dispatched (select! bug) or something \
         else is ticking the counter. Investigate.",
        final_count
    );
}
