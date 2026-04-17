//! TASK-TUI-411: Stuck-parent regression test (TC-TUI-SUBAGENT-05, ERR-TUI-003).
//!
//! ERR-TUI-003 (functional-spec lines 561-566) defines the pre-fix failure:
//! "Subagent spawn delayed because parent is stuck". Post-fix expectation
//! (recovery, 565-566): "Tool-level spawn is independent of parent frame;
//! regression test asserts <10ms spawn under parent blockage."
//!
//! This file is the regression test called out by TECH-TUI-SUBAGENT
//! preserve_invariants (02-technical-spec lines 606-609). It is the final
//! gate for Failure #4 and protects against any future code change that
//! re-introduces a parent-loop indirection for subagent spawn.
//!
//! ------------------------------------------------------------------
//! Marker + shape reconciliation (see stage-5 coverage matrix):
//! ------------------------------------------------------------------
//! 1. Marker: the spec literal says grep for `TASK-TUI-405`. The shipped
//!    code at `archon-core/src/agent.rs:1277` uses `TASK-AGS-105` (the
//!    architectural commit marker under which the removal actually
//!    landed). We grep for the shipped marker; the spec will be
//!    reconciled in Phase C.
//! 2. `ToolResult` shape: the spec literal says `ToolResult::Success(_)
//!    with status == "running"`. `ToolResult` is a struct (`content:
//!    String`, `is_error: bool`) — there is no `Success` enum variant,
//!    and the background-path JSON payload uses `status: "spawned"`
//!    (see `AgentTool::execute` and `task_ags_104.rs::execute_returns_
//!    agent_id_and_spawned_status`). We assert against the shipped
//!    contract (`!is_error` + `status == "spawned"`).
//! 3. Cleanup: spec mentions `cancel_background_agent(&agent_id)` — no
//!    such free function exists in `archon-tools`. Cancellation is via
//!    `BACKGROUND_AGENTS.cancel(&id)`; we use that (matches
//!    `task_ags_104.rs`).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Once;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use archon_tools::agent_tool::{AgentTool, SubagentRequest};
use archon_tools::background_agents::{AgentStatus, BACKGROUND_AGENTS};
use archon_tools::subagent_executor::{
    install_subagent_executor, ExecutorError, OutcomeSideEffects, SubagentClassification,
    SubagentExecutor,
};
use archon_tools::tool::{AgentMode, Tool, ToolContext};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Stub executor (mirrors the TASK-AGS-104 pattern exactly; see
// tests/task_ags_104.rs). The stuck-parent test cares only about the
// synchronous spawn-and-register path, not the downstream runner.
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
        session_id: "task-tui-411-test".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

fn parse_result(content: &str) -> Value {
    serde_json::from_str(content).expect("result.content must be valid JSON")
}

// ---------------------------------------------------------------------------
// TC-TUI-SUBAGENT-05: stuck-parent guard.
//
// Simulate a parent agent task that is blocked on a long `.await`. From
// OUTSIDE that stuck task, call `AgentTool::execute` and assert it
// returns in <10ms (strict, per NFR-TUI-SUB-001 + spec line 566), the
// ToolResult is non-error with `status == "spawned"`, and the agent is
// registered in the global BACKGROUND_AGENTS registry.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tc_tui_subagent_05_stuck_parent_guard() {
    ensure_stub_executor();
    let tool = AgentTool::new();
    let input = json!({
        "prompt": "stuck-parent regression probe",
        "run_in_background": true,
    });

    // Warm-up to page in dashmap + lazy singleton + allocator pages.
    // This matches the TASK-AGS-104 latency-test convention.
    let warm = tool.execute(input.clone(), &make_ctx()).await;
    assert!(!warm.is_error, "warm-up unexpected error: {}", warm.content);
    if let Ok(v) = serde_json::from_str::<Value>(&warm.content) {
        if let Some(id_str) = v["agent_id"].as_str() {
            if let Ok(warm_id) = uuid::Uuid::parse_str(id_str) {
                let _ = BACKGROUND_AGENTS.cancel(&warm_id);
            }
        }
    }

    // Spawn a "stuck" parent task — sleeps 60s, never returns during the
    // test window. Crucially we do NOT await this handle; the test runs
    // on its own scheduler thread (multi_thread flavor, worker_threads=2)
    // so the parent task cannot starve the main test thread.
    let parent = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
    });

    // Invoke the tool WITHOUT awaiting the stuck parent. Measure latency.
    let start = Instant::now();
    let result = tool.execute(input, &make_ctx()).await;
    let elapsed = start.elapsed();

    // Primary assertion: <10ms strict (spec line 566 + NFR-TUI-SUB-001).
    // If this fails, print a clear diagnostic pointing at ERR-TUI-003 so
    // a future regression PR author immediately understands the fault.
    assert!(
        elapsed < Duration::from_millis(10),
        "ERR-TUI-003 regression: AgentTool::execute took {elapsed:?} (>10ms) while parent \
         task is stuck — tool-level spawn must be independent of parent frame. See \
         archon-core/src/agent.rs:1277 (TASK-AGS-105 removal marker) and \
         crates/archon-tools/src/agent_tool.rs::execute."
    );

    // Shape: non-error + parseable JSON payload.
    assert!(
        !result.is_error,
        "unexpected tool error under stuck-parent: {}",
        result.content
    );
    let v = parse_result(&result.content);

    // Contract: background-path payload is {agent_id, status:"spawned"}.
    // Matches the TASK-AGS-104 contract (see
    // tests/task_ags_104.rs::execute_returns_agent_id_and_spawned_status).
    assert_eq!(
        v["status"], "spawned",
        "background-path status field must be 'spawned'; got {v:?}"
    );
    let id_str = v["agent_id"]
        .as_str()
        .expect("agent_id must be a string in the JSON payload");
    let agent_id = uuid::Uuid::parse_str(id_str).expect("agent_id must be a valid uuid-v4");

    // Registry side effect: the spawned agent must be registered.
    // `BACKGROUND_AGENTS.get` returns `Option<AgentStatus>` (not the
    // handle itself). Any non-None status proves registration happened;
    // the stub executor returns immediately so the task may already have
    // flipped to Finished — all three non-Cancelled states are acceptable.
    let status = BACKGROUND_AGENTS.get(&agent_id);
    assert!(
        matches!(
            status,
            Some(AgentStatus::Running)
                | Some(AgentStatus::Finished)
                | Some(AgentStatus::Failed)
        ),
        "subagent must be registered in BACKGROUND_AGENTS after execute; got {status:?}"
    );

    // Cleanup — abort the stuck parent, cancel the subagent registry entry.
    parent.abort();
    let _ = BACKGROUND_AGENTS.cancel(&agent_id);
}

// ---------------------------------------------------------------------------
// Static-analysis guard: archon-core/src/agent.rs must NOT reintroduce
// the deferred `tokio::spawn(...SubagentRequest...)` block that TASK-AGS-105
// removed. This is a cheap grep-in-code check that catches any future PR
// that accidentally resurrects the parent-loop spawn path.
//
// We grep for the shipped marker `TASK-AGS-105` (not the spec-literal
// `TASK-TUI-405`) — see the marker reconciliation note at the top of this
// file. If either the marker disappears OR any single line contains both
// `tokio::spawn` and `SubagentRequest`, the test fails with a clear
// diagnostic naming ERR-TUI-003.
//
// Path construction uses `env!("CARGO_MANIFEST_DIR")` so the test is
// independent of cargo's invocation CWD (spec wiring-check line 88).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tc_tui_subagent_05_no_archon_core_spawn_path() {
    const AGENT_RS_PATH: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/../archon-core/src/agent.rs");

    let src = std::fs::read_to_string(AGENT_RS_PATH).unwrap_or_else(|e| {
        panic!(
            "could not read {AGENT_RS_PATH} for static-analysis guard: {e}. \
             The test is wired to archon-core/src/agent.rs via CARGO_MANIFEST_DIR; \
             if the crate layout changed, update AGENT_RS_PATH."
        )
    });

    // Guard 1: the TASK-AGS-105 marker must still be present. If this
    // disappears, either the removal was reverted or the marker was
    // renamed — both warrant a human review.
    assert!(
        src.contains("TASK-AGS-105"),
        "ERR-TUI-003 regression: archon-core/src/agent.rs no longer contains the \
         TASK-AGS-105 marker. Either the deferred-spawn removal was reverted or \
         the marker text was changed. Review archon-core/src/agent.rs around the \
         subagent tool-call handling site (originally agent.rs:2939-2977)."
    );

    // Guard 2: no single line may contain BOTH `tokio::spawn` AND
    // `SubagentRequest`. The TASK-AGS-105 removal collapsed the old
    // deferred-spawn block; any line matching both substrings is a
    // strong signal the parent-loop spawn path has been reintroduced.
    for (idx, line) in src.lines().enumerate() {
        if line.contains("tokio::spawn") && line.contains("SubagentRequest") {
            panic!(
                "ERR-TUI-003 regression: deferred spawn block reintroduced at \
                 archon-core/src/agent.rs line {} — line contains both \
                 `tokio::spawn` and `SubagentRequest`:\n    {}\n\
                 Tool-level spawn lives in crates/archon-tools/src/agent_tool.rs \
                 per TASK-AGS-105; archon-core must not spawn subagents.",
                idx + 1,
                line.trim()
            );
        }
    }
}
