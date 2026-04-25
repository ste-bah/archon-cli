//! TASK-TUI-409: Integration test for poll_background_agent shim
//! (TC-TUI-SUBAGENT-03). Drift-reconcile to AGS-101 shipped surface
//! via TUI-402 Option A thin shim.
//!
//! 4 spec->shipped reconciliations:
//!   R1: agent_id is Uuid (AGS-101 AgentId alias) -- not String;
//!       extracted from JSON via serde_json::from_str::<Uuid>.
//!   R2: PollOutcome::Complete(AgentStatus) -- not
//!       PollOutcome::Complete(SubagentOutcome). Shim maps shipped
//!       terminal AgentStatus::{Finished, Failed, Cancelled} to the
//!       Complete variant. Spec's "SubagentOutcome is Completed or
//!       Failed" reconciles to AgentStatus::Finished | Failed
//!       (not Cancelled -- which is the explicit reject per spec).
//!   R3: PollOutcome::Running carries no elapsed field -- TUI-402
//!       R2 reconciliation. Spec's elapsed<50ms assertion dropped;
//!       only variant-level assertion remains.
//!   R4: Single-consumption semantics do not apply -- shim is
//!       snapshot-idempotent. Spec's "second call returns Running
//!       because oneshot consumed" contradicted by shipped behavior:
//!       second call returns PollOutcome::Complete(same_status).
//!       Test asserts the SHIPPED behavior.
//!
//! Build requirement: standard test harness; no --release needed
//! (no latency assertions). Test uses AgentTool::execute to drive
//! the registry end-to-end.
//!
//! Fixture pattern: mirrors task_ags_104.rs verbatim -- StubExecutor,
//! ensure_stub_executor, make_ctx, parse_result. No new driver is
//! fabricated. The stub completes run_to_completion immediately with
//! Ok(String::new()), so the spawned task typically resolves to
//! AgentStatus::Finished within the poll loop.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Once;
use std::time::Duration;

use serde_json::{Value, json};

use archon_tools::agent_tool::{AgentTool, SubagentRequest};
use archon_tools::background_agents::AgentStatus;
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
    install_subagent_executor,
};
use archon_tools::tool::{AgentMode, Tool, ToolContext};
use archon_tools::{PollOutcome, cancel_background_agent, poll_background_agent};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Stub executor (pattern reused verbatim from task_ags_104.rs). A no-op
// executor isolates the test to the registry + shim path: execute spawns,
// registers, and the stub's run_to_completion resolves immediately so the
// shipped status transitions Running -> Finished deterministically.
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

    async fn on_inner_complete(&self, _subagent_id: String, _result: Result<String, String>) {}

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
        session_id: "task-tui-409-test".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

fn parse_result(content: &str) -> Value {
    serde_json::from_str(content).expect("result.content must be valid JSON")
}

// ---------------------------------------------------------------------------
// TC-TUI-SUBAGENT-03: poll_background_agent returns Complete(AgentStatus)
// once the spawned stub resolves; the poll is snapshot-idempotent (R4).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tc_tui_subagent_03_poll_returns_completion() {
    ensure_stub_executor();
    let tool = AgentTool::new();
    let input = json!({
        "prompt": "tc-tui-subagent-03 poll-completion probe",
        "run_in_background": true,
    });

    // Step 1-2: drive the tool end-to-end so the registry owns a real
    // background handle. Matches the task_ags_104.rs invocation shape.
    let result = tool.execute(input, &make_ctx()).await;
    assert!(
        !result.is_error,
        "AgentTool::execute must not error on happy path: {}",
        result.content
    );

    // Step 3: extract agent_id as Uuid (R1 reconciliation -- AGS-101 typed).
    let v = parse_result(&result.content);
    let id_str = v["agent_id"]
        .as_str()
        .expect("agent_id must be a string in the JSON payload");
    let agent_id: Uuid =
        serde_json::from_str(&format!("\"{id_str}\"")).expect("agent_id must parse as Uuid");

    // Step 4: immediate poll -- race permitted (Running OR already-Finished
    // because the stub resolves in sub-ms). This is variant-level only;
    // R3 drops any elapsed-field assertion.
    let immediate = poll_background_agent(&agent_id);
    assert!(
        matches!(
            immediate,
            PollOutcome::Running | PollOutcome::Complete(AgentStatus::Finished)
        ),
        "immediate poll must be Running or Complete(Finished); got {immediate:?}"
    );

    // Step 5: poll-loop up to 5 seconds at 10ms cadence until Complete(_).
    let mut terminal: Option<PollOutcome> = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let outcome = poll_background_agent(&agent_id);
        if matches!(outcome, PollOutcome::Complete(_)) {
            terminal = Some(outcome);
            break;
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Step 6: must reach a terminal Complete status. Explicitly reject
    // Cancelled -- the stub completes cleanly, so a Cancelled verdict
    // would indicate the runner was killed, not completed (per R2).
    let terminal = terminal.expect(
        "poll_background_agent must reach PollOutcome::Complete(_) within 5s; \
         stub run_to_completion returns Ok(String::new()) immediately, so \
         this timeout implies a regression in the shim or spawn path",
    );
    let status = match terminal {
        PollOutcome::Complete(s) => s,
        other => panic!(
            "expected PollOutcome::Complete(_) after poll-loop; got {other:?} -- \
             loop was supposed to break only on Complete"
        ),
    };
    assert!(
        matches!(status, AgentStatus::Finished | AgentStatus::Failed),
        "terminal status must be Finished or Failed (R2); Cancelled explicitly \
         rejected -- would mean the runner was killed, not completed. got {status:?}"
    );

    // Step 7: snapshot-idempotent (R4) -- re-polling returns the SAME
    // Complete(status). Spec's "second returns Running because oneshot
    // consumed" is contradicted by shipped behavior.
    let second = poll_background_agent(&agent_id);
    assert_eq!(
        second,
        PollOutcome::Complete(status),
        "R4 snapshot-idempotent: second poll of terminal agent must return \
         the SAME Complete(status). shipped shim does not implement \
         oneshot-consumption semantics."
    );

    // Step 8: cleanup. Ok on live entry; NotFound after reap is acceptable.
    let _ = cancel_background_agent(&agent_id);
}

// ---------------------------------------------------------------------------
// TC-TUI-SUBAGENT-03 (unknown-id branch): poll of a never-registered Uuid
// returns PollOutcome::Unknown. Exercises the R1-typed shim path without
// touching the executor.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tc_tui_subagent_03_poll_unknown_id() {
    let fake_id = Uuid::new_v4();
    assert_eq!(poll_background_agent(&fake_id), PollOutcome::Unknown);
}
