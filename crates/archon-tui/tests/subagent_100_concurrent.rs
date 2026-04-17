//! TASK-TUI-410: Load test TC-TUI-SUBAGENT-04 — 100 concurrent subagents.
//!
//! Implements REQ-TUI-SUB-004 [4/5], NFR-TUI-SUB-001 (<10ms per-spawn),
//! NFR-TUI-SUB-002 (100+ concurrent), AC-SUBAGENT-04, EC-TUI-011.
//!
//! File placed at `crates/archon-tui/tests/subagent_100_concurrent.rs`
//! per TECH-TUI-SUBAGENT implementation_notes line 600-602. The location
//! hint is about CI ownership, not layer placement (spec line 21 /
//! TASK-TUI-410 line 21) — the test drives archon-tools directly via
//! AgentTool, poll_background_agent, cancel_background_agent reached
//! through archon-tools as a dev-dependency.
//!
//! 6 spec→shipped reconciliations (Phase C spec-edit work):
//!   R1: agent_id is Uuid (AGS-101 AgentId alias) — not String. We
//!       extract the string field from JSON then parse via
//!       Uuid::parse_str, matching the pattern in
//!       crates/archon-tools/tests/task_ags_104.rs.
//!   R2: Immediate poll after execute() returns may be
//!       PollOutcome::Running OR PollOutcome::Complete(_). The stub
//!       executor's run_to_completion returns Ok(String::new())
//!       immediately, so a race with the registry reap can flip a
//!       handle to Complete before our first poll. Spec assumed
//!       running-only; reconciled to variant-level acceptance of
//!       Running | Complete(_) (NOT Unknown).
//!   R3: BACKGROUND_AGENTS does not expose .len() on the shipped
//!       surface at the module level; the spec's "registry.len() >= 100"
//!       check is reconciled to "every one of the 100 returned Uuids
//!       is observable (not Unknown) on a poll immediately after the
//!       spawn phase." This is the same operational invariant: all
//!       100 spawned handles exist in the registry. Unknowns = 0 is
//!       the stronger per-id form of len >= 100.
//!   R4: Spec's "status == running" JSON check reconciles to the
//!       AGS-104 contract status == "spawned" (see TUI-404 R1 and
//!       task_ags_104.rs line 113). AgentTool::execute returns
//!       {"agent_id": "<uuid>", "status": "spawned"} synchronously;
//!       "running" is a lifecycle state visible via PollOutcome, not
//!       a JSON field.
//!   R5: Completion detection via poll_background_agent loop until
//!       PollOutcome::Complete(_) OR PollOutcome::Unknown. Unknown
//!       is treated as a terminal observation because reap_finished
//!       called by any caller will drop the entry — at that point
//!       the handle is guaranteed to have been in a terminal state
//!       before the reap. Both count as "done" for the 100/100
//!       completion bound.
//!   R6: Cleanup via cancel_background_agent; NotFound on already-
//!       reaped entries is ignored (RegistryError::NotFound does not
//!       fail the test).
//!
//! Build requirement: --release (per TASK-TUI-410.md line 42;
//! NFR-TUI-SUB-001 <10ms per-spawn latency requires an optimized
//! build to be meaningful — debug builds add ~10x overhead to JSON
//! serialization + DashMap inserts and would falsely trip the bound).
//!
//! Fixture pattern: mirrors crates/archon-tools/tests/task_ags_104.rs
//! and crates/archon-tools/tests/poll_background_agent.rs — StubExecutor,
//! ensure_stub_executor, make_ctx, parse_result — reused verbatim. All
//! types and install_subagent_executor are `pub` in archon-tools so the
//! fixture is reachable from a sibling dev-dependency test crate with
//! no feature-flag gymnastics.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Once;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use archon_tools::agent_tool::{AgentTool, SubagentRequest};
use archon_tools::background_agents::AgentStatus;
use archon_tools::subagent_executor::{
    install_subagent_executor, ExecutorError, OutcomeSideEffects, SubagentClassification,
    SubagentExecutor,
};
use archon_tools::tool::{AgentMode, Tool, ToolContext};
use archon_tools::{cancel_background_agent, poll_background_agent, PollOutcome};
use async_trait::async_trait;
use futures::future::join_all;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Stub executor (pattern reused verbatim from task_ags_104.rs). A no-op
// executor isolates the test to the spawn + registry + shim path. The
// stub's run_to_completion resolves immediately with Ok(String::new()),
// so the shipped AgentStatus transitions Running -> Finished without
// performing any real agent I/O — keeping the per-spawn cost dominated
// by the registry insert that NFR-TUI-SUB-001 actually targets.
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
        session_id: "task-tui-410-test".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

fn parse_result(content: &str) -> Value {
    serde_json::from_str(content).expect("result.content must be valid JSON")
}

// ---------------------------------------------------------------------------
// TC-TUI-SUBAGENT-04: 100 concurrent spawns via join_all, per-spawn
// latency <10ms, all 100 observable in the registry, all 100 reach
// terminal observation (Complete or Unknown-post-reap) within 30s, and
// every agent_id cleaned up on exit.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn tc_tui_subagent_04_100_concurrent() {
    ensure_stub_executor();
    let tool = AgentTool::new();

    // Pre-warm: one execute() call, discard + cleanup. Same rationale as
    // TASK-TUI-408 cold-start note — first execute() pages in DashMap
    // lazy allocs + serde_json heap setup + the subagent_executor
    // OnceLock, none of which are fair to charge to the 100 concurrent
    // calls we're measuring.
    {
        let warm_input = json!({
            "prompt": "tc-tui-subagent-04 pre-warm",
            "run_in_background": true,
        });
        let warm_result = tool.execute(warm_input, &make_ctx()).await;
        assert!(
            !warm_result.is_error,
            "pre-warm execute must succeed: {}",
            warm_result.content
        );
        if let Ok(v) = serde_json::from_str::<Value>(&warm_result.content) {
            if let Some(id_str) = v["agent_id"].as_str() {
                if let Ok(warm_id) = Uuid::parse_str(id_str) {
                    let _ = cancel_background_agent(&warm_id);
                }
            }
        }
    }

    // Build 100 distinct inputs. Distinct prompts ensure no caller-side
    // deduplication could coalesce calls (not that AgentTool does any,
    // but makes the intent explicit).
    let mut futures_vec = Vec::with_capacity(100);
    for i in 0..100u32 {
        let tool_ref = &tool;
        let ctx = make_ctx();
        let input = json!({
            "prompt": format!("tc-tui-subagent-04 concurrent probe {i}"),
            "run_in_background": true,
        });
        futures_vec.push(async move {
            let start = Instant::now();
            let result = tool_ref.execute(input, &ctx).await;
            (start.elapsed(), result)
        });
    }

    // Await all 100 concurrently via join_all. We rely on the multi_thread
    // tokio runtime + join_all's polled-in-order-but-concurrent semantics
    // to give a genuine 100-way fan-out.
    let outcomes = join_all(futures_vec).await;
    assert_eq!(outcomes.len(), 100, "expected 100 outcomes from join_all");

    // ----- Assertion 1: per-spawn latency <10ms for every call. -----
    let mut elapsed_samples: Vec<Duration> = outcomes.iter().map(|(e, _)| *e).collect();
    let over_budget: Vec<(usize, Duration)> = elapsed_samples
        .iter()
        .enumerate()
        .filter(|(_, e)| **e >= Duration::from_millis(10))
        .map(|(i, e)| (i, *e))
        .collect();
    if !over_budget.is_empty() {
        elapsed_samples.sort();
        let max = elapsed_samples.last().copied().unwrap_or_default();
        // p99 on 100 samples = index 98 after sort (0-indexed, take 99th value)
        let p99 = elapsed_samples.get(98).copied().unwrap_or_default();
        panic!(
            "NFR-TUI-SUB-001 violation: {} of 100 calls exceeded 10ms. \
             max={max:?}, p99={p99:?}. first 5 offenders: {:?}",
            over_budget.len(),
            &over_budget.iter().take(5).collect::<Vec<_>>(),
        );
    }

    // ----- Assertion 2: every result is Success with status "spawned" (R4). -----
    let mut ids: Vec<Uuid> = Vec::with_capacity(100);
    for (i, (_elapsed, result)) in outcomes.iter().enumerate() {
        assert!(
            !result.is_error,
            "call {i} returned is_error=true: {}",
            result.content
        );
        let v = parse_result(&result.content);
        assert_eq!(
            v["status"], "spawned",
            "call {i} status must be 'spawned' (AGS-104 contract, R4); got {}",
            v["status"]
        );
        let id_str = v["agent_id"]
            .as_str()
            .unwrap_or_else(|| panic!("call {i} missing agent_id string field: {v:?}"));
        let id = Uuid::parse_str(id_str)
            .unwrap_or_else(|e| panic!("call {i} agent_id not a valid uuid: {id_str}: {e}"));
        ids.push(id);
    }

    // ----- Assertion 3: all 100 Uuids are distinct. -----
    let unique: HashSet<Uuid> = ids.iter().copied().collect();
    assert_eq!(
        unique.len(),
        100,
        "expected 100 distinct agent_ids; got {} unique",
        unique.len(),
    );

    // ----- Assertion 4 (R3): every Uuid observable immediately — NOT Unknown. -----
    // This is the reconciled form of "BACKGROUND_AGENTS.len() >= 100":
    // per-id observability proves every spawn registered.
    for (i, id) in ids.iter().enumerate() {
        let outcome = poll_background_agent(id);
        assert!(
            matches!(outcome, PollOutcome::Running | PollOutcome::Complete(_)),
            "id {i} ({id}) must be Running or Complete immediately post-spawn; \
             Unknown means registration failed. got {outcome:?}"
        );
    }

    // ----- Completion wait (R5): poll-loop up to 30s at 50ms cadence. -----
    // Terminal observation per-id = Complete(_) OR Unknown. Unknown means
    // the entry has been reaped — which only happens after a terminal
    // state — so it is equivalent to Complete for bound purposes.
    let mut done: HashSet<Uuid> = HashSet::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    while done.len() < 100 {
        for id in &ids {
            if done.contains(id) {
                continue;
            }
            match poll_background_agent(id) {
                PollOutcome::Complete(_) | PollOutcome::Unknown => {
                    done.insert(*id);
                }
                PollOutcome::Running => {}
            }
        }
        if done.len() >= 100 {
            break;
        }
        if Instant::now() >= deadline {
            // Diagnostic: how many are still Running vs already done.
            let still_running: Vec<Uuid> =
                ids.iter().copied().filter(|id| !done.contains(id)).collect();
            panic!(
                "AC-SUBAGENT-04 completion timeout: only {}/100 reached terminal \
                 observation within 30s. {} still Running. first 5 stuck: {:?}",
                done.len(),
                still_running.len(),
                still_running.iter().take(5).collect::<Vec<_>>(),
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        done.len(),
        100,
        "expected 100/100 terminal observations (Complete or Unknown-post-reap)"
    );

    // Pre-cleanup sampling: confirm AgentStatus variants we actually saw
    // fall in the Finished | Failed band (not Cancelled — stub completes
    // cleanly). MUST run BEFORE the cleanup cancel loop — otherwise cancel
    // unconditionally writes AgentStatus::Cancelled at background_agents.rs:204,
    // which would flip Finished entries to Cancelled and break the assertion.
    // (Gate 4 remediation: original ordering had sampling AFTER cleanup
    // and panicked deterministically under StubExecutor.)
    let mut any_terminal_inspected = false;
    for id in ids.iter().take(5) {
        if let PollOutcome::Complete(status) = poll_background_agent(id) {
            any_terminal_inspected = true;
            assert!(
                matches!(status, AgentStatus::Finished | AgentStatus::Failed),
                "sampled terminal status must be Finished or Failed; \
                 Cancelled would indicate the runner was killed (R2 parallel \
                 of TUI-409 reasoning). got {status:?}"
            );
        }
    }
    let _ = any_terminal_inspected;

    // ----- Cleanup (R6): ignore NotFound on already-reaped entries. -----
    // We deliberately do NOT assert success here — a handle reaped by
    // any background sweep is expected to error with NotFound, which is
    // not a failure for this test.
    for id in &ids {
        let _ = cancel_background_agent(id);
    }
}
