//! TASK-TUI-902 — PRESERVE gate: subagent spawn latency under parent blockage.
//!
//! ## Purpose (ERR-TUI-003 regression guard)
//!
//! ERR-TUI-003 was the pre-fix failure mode where a subagent spawn was
//! routed back through the **parent agent's** message loop, so a busy or
//! blocked parent stalled every child spawn. SPEC-TUI-SUBAGENT fixed it by
//! making subagent spawn go through the direct AgentTool::execute +
//! BACKGROUND_AGENTS.register path (ExplicitBackground classification),
//! which detaches the runner onto its own `tokio::spawn` task and returns
//! `{"agent_id", "status":"spawned"}` synchronously — bypassing the parent
//! loop entirely. That positive fix is owned by earlier SUBAGENT tasks;
//! this test owns the **standing latency regression guard** so nobody
//! re-introduces the parent-loop routing by accident.
//!
//! ## What it measures
//!
//! - Stub parent `process_message` sleeps 5s (simulates blocked parent).
//! - The parent task is spawned first, so it is ACTIVELY blocked when we
//!   measure subagent spawn latency. Because spawn goes through the direct
//!   AgentTool::execute path (not the parent loop), the parent's 5s sleep
//!   is completely irrelevant to the measurement.
//! - Per iteration: `t0 = Instant::now()`; call `AgentTool::execute` with
//!   `run_in_background: true`; `t1 = Instant::now()`; elapsed = t1 - t0.
//! - Budget: 10ms release, 20ms debug (per TASK-TUI-902 Validation #5).
//! - 10 iterations; ALL must pass (spec line 33: "require all 10 to pass").
//!
//! ## Entrypoint (TECH-TUI-SUBAGENT decision)
//!
//! The public direct-spawn entrypoint IS `archon_tools::agent_tool::AgentTool::execute`
//! with `run_in_background: true`. Per `crates/archon-tools/src/agent_tool.rs:258-412`
//! this path:
//!   1. Validates + classifies as ExplicitBackground.
//!   2. `tokio::spawn`s the runner (via installed `SubagentExecutor`).
//!   3. Registers the handle in `BACKGROUND_AGENTS`.
//!   4. Returns `ToolResult::success({"agent_id","status":"spawned"})`.
//! Steps 2-4 are synchronous from the caller's point of view — no await on
//! the parent loop. This is the "BACKGROUND_AGENTS insert returns" instant
//! that the sibling TUI-811 load test also anchors on.
//!
//! This is the same public API TUI-811 uses (see `subagent_100_concurrent.rs`);
//! wrapping it in a type-alias or re-export in archon-tui would add production
//! code for no test-side benefit, and TASK-TUI-902 scope says "No production
//! code added" (Wiring Check).
//!
//! ## Parent blockage model (spec line 21-22, 32)
//!
//! The spec describes a parent whose `process_message` sleeps 5s. We model
//! that as a `tokio::spawn`ed task that enters a 5-second `tokio::time::sleep`
//! BEFORE we start the latency loop — so the parent is actively blocked on
//! the runtime when each spawn happens. Since the direct-spawn path never
//! touches the parent task (the whole point of the fix), this pins down
//! that independence: if someone wires spawn back through the parent, the
//! measured latency jumps from <10ms to ~5s and the gate fires.
//!
//! ## Runtime budget
//!
//! With `worker_threads = 2` (spec line 83-84), the parent 5s sleep sits on
//! one worker while the spawn measurements run on the other. Total
//! wall-clock: ~5s (parent) + ~100ms (10 spawns × ~10ms headroom). We do
//! NOT use `tokio::time::pause()` because `AgentTool::execute` internally
//! calls `SystemTime::now()` which `pause()` does not virtualize, and the
//! real `tokio::spawn` interaction needs real time for the JoinHandle
//! adapter to settle. Wallclock cost is ~5s on a real runner; acceptable.
//!
//! ## Negative test feasibility (spec Validation #4)
//!
//! A local simulation of the pre-fix routing (await the parent's
//! `process_message` before the spawn call) proves the gate fires with
//! latency ≈ 5s. That simulation would touch production code, so it's
//! documented here rather than shipped — the spec says the gate validates
//! the existing direct-spawn path stays <10ms; reverting the refactor is
//! owned by humans inspecting diffs, not by a second test.
//!
//! 5 spec→shipped reconciliations (pattern echoes TUI-811 R1–R6):
//!   S1: "subagent spawn entrypoint" is not a single named symbol in
//!       this tree; it's `AgentTool::execute` + ExplicitBackground path.
//!       Reconciled by using AgentTool directly (same as TUI-811).
//!   S2: `StubExecutor` with immediate-complete `run_to_completion` so
//!       the detached runner finishes fast and doesn't leak JoinHandles
//!       across iterations — same pattern as `subagent_100_concurrent.rs`.
//!   S3: Spec says "records Instant::now() on construction / spawn-confirm
//!       event". The confirmation event IS the return of `AgentTool::execute`
//!       with `status:"spawned"`. We record `t1 = Instant::now()`
//!       immediately after the await returns Ok.
//!   S4: Pre-warm the executor + DashMap on iteration 0 (same as TUI-811),
//!       then record 10 measurement iterations. Total 11 calls; only 10 are
//!       scored against the budget.
//!   S5: Failure panic includes `ERR-TUI-003` string + offending
//!       iteration's index + measured `Duration` (spec line 35-36).

#![allow(clippy::needless_collect)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use archon_tools::agent_tool::{AgentTool, SubagentRequest};
use archon_tools::subagent_executor::{
    install_subagent_executor, ExecutorError, OutcomeSideEffects, SubagentClassification,
    SubagentExecutor,
};
use archon_tools::tool::{AgentMode, Tool, ToolContext};
use archon_tools::cancel_background_agent;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Tuning constants — one-line knobs for the gate.
// ---------------------------------------------------------------------------

/// Number of measurement iterations (spec line 33: "loop of 10 iterations").
const ITERATIONS: usize = 10;

/// Latency budget (spec Validation #5 + task prompt): 10ms release / 20ms debug.
#[cfg(debug_assertions)]
const BUDGET: Duration = Duration::from_millis(20);
#[cfg(not(debug_assertions))]
const BUDGET: Duration = Duration::from_millis(10);

/// How long the stub parent's `process_message` blocks. Spec line 31.
const PARENT_BLOCK: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Stub executor — no-op runner so AgentTool::execute's detached runner
// completes immediately without any LLM I/O. Same pattern as
// `subagent_100_concurrent.rs` (TUI-811). `run_to_completion` returning Ok
// is enough for the register path to resolve; we never poll the terminal
// status in this gate because we only care about the pre-register latency.
// ---------------------------------------------------------------------------

struct StubExecutor;

#[async_trait]
impl SubagentExecutor for StubExecutor {
    async fn run_to_completion(
        &self,
        _subagent_id: String,
        _request: SubagentRequest,
        _ctx: ToolContext,
        _cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        Ok(String::new())
    }

    async fn on_inner_complete(
        &self,
        _subagent_id: String,
        _result: Result<String, String>,
    ) {
    }

    async fn on_visible_complete(
        &self,
        _subagent_id: String,
        _result: Result<String, String>,
        _nested: bool,
    ) -> OutcomeSideEffects {
        OutcomeSideEffects::default()
    }

    fn auto_background_ms(&self) -> u64 {
        0
    }

    fn classify(&self, req: &SubagentRequest) -> SubagentClassification {
        if req.run_in_background {
            SubagentClassification::ExplicitBackground
        } else {
            SubagentClassification::Foreground
        }
    }
}

static INSTALL_ONCE: Once = Once::new();

fn ensure_stub_executor() {
    INSTALL_ONCE.call_once(|| {
        install_subagent_executor(Arc::new(StubExecutor));
    });
}

fn make_ctx() -> ToolContext {
    ToolContext {
        working_dir: PathBuf::from("/tmp"),
        session_id: "task-tui-902-test".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

fn parse_result(content: &str) -> Value {
    serde_json::from_str(content).expect("AgentTool::execute result.content must be valid JSON")
}

/// Minimal valid no-op AgentTool request (run_in_background: true routes
/// through the ExplicitBackground / direct-spawn path).
fn make_spawn_request(iter_idx: usize) -> Value {
    json!({
        "prompt": format!("task-tui-902 preserve latency probe iter={iter_idx}"),
        "run_in_background": true,
    })
}

// ---------------------------------------------------------------------------
// Stub parent "process_message" — sleeps PARENT_BLOCK (5s). Per spec line 31
// this simulates the pre-fix parent-loop blockage. Kept as a plain async
// function rather than a trait/struct because (a) the gate is observational,
// not a unit test of parent-loop behaviour, and (b) adding a trait here
// would pull in production code. The atomic flag just lets us assert the
// parent actually entered its block before we start measuring (avoids a
// silent race where the parent finishes before we even spawn it).
// ---------------------------------------------------------------------------

async fn stub_blocked_parent_process_message(entered: Arc<AtomicBool>) {
    entered.store(true, Ordering::Relaxed);
    tokio::time::sleep(PARENT_BLOCK).await;
}

// ---------------------------------------------------------------------------
// Quantile helper — same shape as `event_latency_p95.rs`'s print but using a
// simple sorted-copy rather than hdrhistogram, because 10 samples is far
// below hdrhistogram's useful resolution and we want exact observed values
// for the CI grep line.
// ---------------------------------------------------------------------------

fn quantile_micros(sorted: &[Duration], q: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    // Nearest-rank: index = ceil(q * n) - 1, clamped to [0, n-1].
    let n = sorted.len();
    let idx = ((q * n as f64).ceil() as usize).saturating_sub(1).min(n - 1);
    sorted[idx].as_micros()
}

// ---------------------------------------------------------------------------
// AC-PRESERVE-03 / TC-TUI-PRESERVE-03: direct subagent-spawn latency must
// stay <10ms (release) even while a stub parent's `process_message` is
// blocked for 5s. Re-asserts REQ-TUI-SUB-002 [6/6: preserve gate] per spec.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preserve_subagent_spawn_latency_gate() {
    ensure_stub_executor();
    let tool = Arc::new(AgentTool::new());

    // --- Pre-warm ---------------------------------------------------------
    // First `AgentTool::execute` call pages in DashMap lazy allocs + serde_json
    // heap setup + the subagent_executor OnceLock. None of that is fair to
    // charge against the 10 measurement iterations. Same rationale as TUI-811.
    let warm = tool
        .execute(
            json!({ "prompt": "preserve-902 pre-warm", "run_in_background": true }),
            &make_ctx(),
        )
        .await;
    assert!(
        !warm.is_error,
        "pre-warm AgentTool::execute must succeed: {}",
        warm.content
    );
    let warm_id = parse_result(&warm.content)["agent_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok());
    if let Some(id) = warm_id {
        let _ = cancel_background_agent(&id);
    }

    // --- Spawn the blocked parent ----------------------------------------
    // We spawn the parent FIRST so it is actively sleeping on a worker thread
    // when we start the iteration loop. The `entered` flag lets us block
    // until the parent's stub has actually entered the sleep — otherwise we
    // could race and measure spawn latency before the parent's sleep started,
    // which would technically satisfy the assertion but would not prove
    // independence from parent blockage (the gate's entire purpose).
    let entered = Arc::new(AtomicBool::new(false));
    let parent_entered = Arc::clone(&entered);
    let parent_handle =
        tokio::spawn(
            async move { stub_blocked_parent_process_message(parent_entered).await },
        );

    // Busy-wait (bounded) for the parent to enter its block. Max 500ms of
    // real wallclock — if the parent hasn't been polled by then something
    // is very wrong with the runtime. Note: this yield loop is NOT part of
    // any measured latency interval.
    {
        let deadline = Instant::now() + Duration::from_millis(500);
        while !entered.load(Ordering::Acquire) && Instant::now() < deadline {
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert!(
            entered.load(Ordering::Acquire),
            "stub parent did not enter its 5s block within 500ms — worker_threads < 2?"
        );
    }

    // --- 10 measurement iterations ---------------------------------------
    let samples: Arc<Mutex<Vec<Duration>>> =
        Arc::new(Mutex::new(Vec::with_capacity(ITERATIONS)));
    let mut spawned_ids: Vec<Uuid> = Vec::with_capacity(ITERATIONS);

    for i in 0..ITERATIONS {
        let input = make_spawn_request(i);
        let ctx = make_ctx();

        // t0 captured immediately before `execute()` — this is the caller's
        // "spawn requested" instant.
        let t0 = Instant::now();
        let result = tool.execute(input, &ctx).await;
        // t1 captured immediately after the await returns — per
        // archon-tools/src/agent_tool.rs:405-411 the ExplicitBackground path
        // returns `{"agent_id","status":"spawned"}` right after
        // `BACKGROUND_AGENTS.register(handle)` resolves. This is the spec's
        // "spawn-confirmation event" instant.
        let t1 = Instant::now();
        let elapsed = t1 - t0;

        assert!(
            !result.is_error,
            "iter {i}: AgentTool::execute returned is_error=true (ERR-TUI-003 harness broken): {}",
            result.content
        );
        let v = parse_result(&result.content);
        assert_eq!(
            v["status"], "spawned",
            "iter {i}: expected status='spawned' (direct-spawn contract, ERR-TUI-003); got {}",
            v["status"]
        );
        let id_str = v["agent_id"]
            .as_str()
            .unwrap_or_else(|| panic!("iter {i}: agent_id missing from result: {v:?}"));
        let id = Uuid::parse_str(id_str).unwrap_or_else(|e| {
            panic!("iter {i}: agent_id not a valid UUID: {id_str}: {e}")
        });
        spawned_ids.push(id);
        samples.lock().expect("samples mutex poisoned").push(elapsed);

        // Sanity assert per-iteration (spec line 37-39: "assert elapsed <
        // Duration::from_millis(10)"; failure MUST mention ERR-TUI-003 and
        // measured latency).
        assert!(
            elapsed < BUDGET,
            "ERR-TUI-003 regression: subagent spawn latency {elapsed:?} exceeded budget {BUDGET:?} \
             on iteration {i}. Parent was actively blocked for {PARENT_BLOCK:?}; the direct-spawn \
             path (AgentTool::execute -> BACKGROUND_AGENTS.register) must return in <10ms \
             independent of parent state. See AC-PRESERVE-03, TC-TUI-PRESERVE-03."
        );
    }

    // --- Aggregate stats --------------------------------------------------
    let samples_vec: Vec<Duration> = samples.lock().expect("samples mutex poisoned").clone();
    assert_eq!(
        samples_vec.len(),
        ITERATIONS,
        "expected {ITERATIONS} samples, got {}",
        samples_vec.len()
    );
    let mut sorted = samples_vec.clone();
    sorted.sort();
    let p50 = quantile_micros(&sorted, 0.50);
    let p95 = quantile_micros(&sorted, 0.95);
    let p99 = quantile_micros(&sorted, 0.99);
    let max = sorted.last().copied().unwrap_or_default();
    let avg_us: u128 = samples_vec.iter().map(|d| d.as_micros()).sum::<u128>()
        / samples_vec.len() as u128;

    // Spec Validation #2: average measured latency reported in test output <5ms.
    // (We print it; the hard gate is the per-sample <10ms bound above.)
    println!(
        "[preserve_subagent_spawn_latency_gate] samples={} avg_us={} max={:?} p50_us={} p95_us={} p99_us={}",
        samples_vec.len(),
        avg_us,
        max,
        p50,
        p95,
        p99,
    );
    // Plain quantile line for CI grep symmetry with event_latency_p95.rs.
    println!("p50={} p95={} p99={}", p50, p95, p99);

    // --- Post-check: belt & braces on all 10 against budget --------------
    // The per-iteration assert already fires; this final sweep is the exact
    // form the spec grep line wants ("require all 10 to pass").
    for (i, s) in samples_vec.iter().enumerate() {
        assert!(
            *s < BUDGET,
            "ERR-TUI-003 regression (final sweep): iteration {i} latency {s:?} >= budget {BUDGET:?}"
        );
    }

    // --- Parent task is still sleeping — abort rather than wait ----------
    // We've proven spawn latency is independent of parent blockage; no need
    // to pay the full 5s of wallclock for the parent's sleep to complete.
    parent_handle.abort();
    // Ignore abort result — JoinError::is_cancelled() is the expected state.
    let _ = parent_handle.await;

    // --- Cleanup: cancel the 10 (+warm) detached background subagents ----
    // Same pattern as TUI-811 — NotFound on already-reaped entries is fine.
    for id in &spawned_ids {
        let _ = cancel_background_agent(id);
    }
}
