//! The inactivity bound end to end: `SubagentPipelineClient::run_agent` ->
//! foreground spawn -> executor, with the clock crossing the spawn the way the
//! runner reads it. The executor stands in for the runner and reports activity
//! exactly where the runner does.

use std::sync::Arc;
use std::time::Duration;

use archon_tools::subagent_activity;
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
    install_subagent_executor,
};
use archon_tools::subagent_request::SubagentRequest;
use archon_tools::tool::ToolContext;
use tokio_util::sync::CancellationToken;

use crate::runner::{AgentExecutionRequest, LlmClient, PipelineType, ToolAccessLevel};
use crate::subagent_adapter::SubagentPipelineClient;
use crate::subagent_adapter::tests::{NoopClient, request};

const BOUND: u64 = 1_800;

/// Behaves by the prompt, so every test here can install the same executor
/// without depending on which one the process holds.
struct ScriptedRunner;

#[async_trait::async_trait]
impl SubagentExecutor for ScriptedRunner {
    async fn run_to_completion(
        &self,
        _: String,
        _: SubagentRequest,
        _: ToolContext,
        _: CancellationToken,
    ) -> Result<String, ExecutorError> {
        Err(ExecutorError::Internal("system path required".into()))
    }

    async fn run_to_completion_with_system(
        &self,
        _: String,
        request: SubagentRequest,
        _: Vec<serde_json::Value>,
        _: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        if request.prompt.contains("NO-CLOCK-EXPECTED") {
            assert!(subagent_activity::current().is_none(), "bound is off");
            tokio::time::sleep(Duration::from_secs(BOUND * 5)).await;
            return Ok("finished unbounded".into());
        }
        assert!(
            subagent_activity::current().is_some(),
            "the clock did not cross the executor spawn"
        );
        if request.prompt.contains("SLOW-TOOLS") {
            // Case B: every tool call is slow, but the calls keep coming.
            for _ in 0..6 {
                subagent_activity::note();
                let round = subagent_activity::tool_round();
                tokio::time::sleep(Duration::from_secs(BOUND - 100)).await;
                drop(round);
            }
            return Ok("reviewed".into());
        }
        // Case A: one stuck request; nothing more ever arrives.
        subagent_activity::note();
        cancel.cancelled().await;
        Err(ExecutorError::Internal(
            "Subagent cancelled during LLM inference".into(),
        ))
    }

    async fn on_inner_complete(&self, _: String, _: Result<String, String>) {}

    async fn on_visible_complete(
        &self,
        _: String,
        _: Result<String, String>,
        _: bool,
    ) -> OutcomeSideEffects {
        Default::default()
    }

    fn auto_background_ms(&self) -> u64 {
        0
    }

    fn classify(&self, _: &SubagentRequest) -> SubagentClassification {
        SubagentClassification::Foreground
    }
}

fn workflow_request(prompt: &str) -> AgentExecutionRequest {
    let mut request = request(ToolAccessLevel::ReadOnly);
    request.session_id = format!("inactivity-{prompt}");
    request.pipeline_type = PipelineType::Workflow;
    request.messages = vec![serde_json::json!({"role": "user", "content": prompt})];
    request.timeout_secs = Some(14_400);
    request.disable_auto_background = true;
    request
}

fn client(bound: Option<u64>) -> SubagentPipelineClient {
    install_subagent_executor(Arc::new(ScriptedRunner));
    SubagentPipelineClient::new(Arc::new(NoopClient), ToolContext::default())
        .with_inactivity_timeout(bound)
}

#[tokio::test(start_paused = true)]
async fn a_stalled_session_is_cut_for_inactivity_through_the_real_spawn_path() {
    let started = tokio::time::Instant::now();
    let error = client(Some(BOUND))
        .run_agent(workflow_request("STALL"))
        .await
        .expect_err("a stalled session is cut");
    let text = error.to_string();
    assert!(
        subagent_activity::is_inactivity_timeout_text(&text),
        "{text}"
    );
    assert!(!text.contains("timed out after"), "{text}");
    let elapsed = tokio::time::Instant::now() - started;
    assert!(
        elapsed >= Duration::from_secs(BOUND) && elapsed < Duration::from_secs(BOUND + 60),
        "cut at the bound, hours before the 14400s wall clock: {elapsed:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_slow_but_active_session_runs_to_completion_through_the_real_spawn_path() {
    let response = client(Some(BOUND))
        .run_agent(workflow_request("SLOW-TOOLS"))
        .await
        .expect("continuous tool calls are activity");
    assert_eq!(response.content, "reviewed");
}

#[tokio::test(start_paused = true)]
async fn a_disabled_bound_installs_no_clock_through_the_real_spawn_path() {
    let response = client(Some(0))
        .run_agent(workflow_request("NO-CLOCK-EXPECTED"))
        .await
        .expect("off means off");
    assert_eq!(response.content, "finished unbounded");
}
