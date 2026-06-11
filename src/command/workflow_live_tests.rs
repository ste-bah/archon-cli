use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use archon_pipeline::runner::{AgentExecutionRequest, LlmClient, LlmResponse};
use archon_workflow::WorkflowStageRunner;
use serde_json::json;

use super::workflow_live_retry::transient_live_agent_error;
use super::{
    PipelineWorkflowRunner, ProviderTier, StageKind, StageRunRequest, allowed_tools, extract_yaml,
    plan_live, request_target_repository_root, workflow_prompt,
};

struct InvalidPlanner;

struct FlakyAgentClient {
    calls: AtomicUsize,
    first_error: &'static str,
}

#[async_trait::async_trait]
impl LlmClient for InvalidPlanner {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        Ok(LlmResponse {
            // Genuinely unrecoverable: a stage pins a concrete model, which
            // validate_stage_fields rejects and the normalizer does not touch.
            content: r#"
schema: archon.workflow.v1
name: invalid-live-plan
task: implement a real task
stages:
  - id: discover
    kind: agent
    provider_tier: planner
    model: claude-opus-4-8
"#
            .to_string(),
            tool_uses: Vec::new(),
            tokens_in: 0,
            tokens_out: 0,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for FlakyAgentClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        anyhow::bail!("test should use run_agent");
    }

    async fn run_agent(&self, _request: AgentExecutionRequest) -> Result<LlmResponse> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            anyhow::bail!(self.first_error);
        }
        Ok(LlmResponse {
            content: "status: completed".to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

fn request(input: serde_json::Value) -> StageRunRequest {
    StageRunRequest {
        run_id: "wf-test".into(),
        stage_id: "implement".into(),
        stage_kind: StageKind::Implementation,
        agent: None,
        task: "implement".into(),
        attempt: 1,
        provider_tier: ProviderTier::Coder,
        depends_on: Vec::new(),
        input,
    }
}

fn runner(llm: Arc<dyn LlmClient>) -> PipelineWorkflowRunner {
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(16);
    PipelineWorkflowRunner {
        llm,
        tui_tx,
        agent_names: Vec::new(),
    }
}

#[test]
fn workflow_live_uses_target_repository_root_as_subagent_cwd() {
    let req = request(json!({
        "target_repository_root": "/tmp/target-repo",
    }));

    assert_eq!(
        request_target_repository_root(&req),
        Some(PathBuf::from("/tmp/target-repo"))
    );
}

#[test]
fn workflow_live_omits_empty_target_repository_root() {
    let req = request(json!({
        "target_repository_root": " ",
    }));

    assert_eq!(request_target_repository_root(&req), None);
}

#[test]
fn extract_yaml_accepts_plain_or_fenced_output() {
    assert_eq!(
        extract_yaml("```yaml\nschema: archon.workflow.v1\n```\n"),
        "schema: archon.workflow.v1"
    );
    assert_eq!(
        extract_yaml("schema: archon.workflow.v1\n"),
        "schema: archon.workflow.v1"
    );
}

#[test]
fn focused_test_workflow_stages_can_execute_commands_without_write_tools() {
    let req = StageRunRequest {
        stage_id: "focused_tests-8".into(),
        stage_kind: StageKind::Agent,
        task: "Run focused cargo test evidence for TASK-TRL-011".into(),
        ..request(json!({}))
    };
    let tools = allowed_tools(&req);

    assert!(tools.contains(&"Bash".to_string()));
    assert!(tools.contains(&"Read".to_string()));
    assert!(!tools.contains(&"Write".to_string()));
    assert!(!tools.contains(&"Edit".to_string()));
}

#[test]
fn post_remediation_test_stages_can_execute_commands_without_write_tools() {
    let req = StageRunRequest {
        stage_id: "wave2_post_tests".into(),
        stage_kind: StageKind::Agent,
        task: "Run focused post-remediation tests for T010/T020/T030 and capture exact commands/results.".into(),
        ..request(json!({}))
    };
    let tools = allowed_tools(&req);

    assert!(tools.contains(&"Bash".to_string()));
    assert!(tools.contains(&"Read".to_string()));
    assert!(!tools.contains(&"Write".to_string()));
    assert!(!tools.contains(&"Edit".to_string()));
}

#[test]
fn command_stage_prompt_uses_configured_bash_timeout() {
    let req = StageRunRequest {
        stage_id: "wave2_post_tests".into(),
        stage_kind: StageKind::Agent,
        task: "Run focused post-remediation tests for T010/T020/T030 and capture exact commands/results.".into(),
        ..request(json!({}))
    };
    let prompt = workflow_prompt(&req);

    assert!(prompt.contains("rely on the configured `tools.bash_timeout`"));
    assert!(prompt.contains("Do not set a Bash `timeout` field"));
    assert!(prompt.contains("do not wrap commands with shell-level `timeout`/`gtimeout`"));
    assert!(prompt.contains("Do not mark timed-out commands as completed or verified"));
}

#[test]
fn explicit_stage_extra_can_request_bash() {
    let req = StageRunRequest {
        stage_id: "validate".into(),
        task: "Validate generated outputs".into(),
        ..request(json!({
            "stage_extra": {
                "allowed_tools": ["Read", "Bash"]
            }
        }))
    };

    assert!(allowed_tools(&req).contains(&"Bash".to_string()));
}

#[tokio::test]
async fn live_planner_validation_failure_does_not_fallback_to_smoke_plan() {
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(16);
    let err = plan_live("implement the whole PRD", Arc::new(InvalidPlanner), tui_tx)
        .await
        .expect_err("invalid live plans must fail instead of using heuristic fallback");
    assert!(!err.to_string().is_empty());
}

#[tokio::test]
async fn workflow_live_retries_transient_agent_decode_errors() {
    let client = Arc::new(FlakyAgentClient {
        calls: AtomicUsize::new(0),
        first_error: "HTTP error: http_error: HTTP error: error decoding response body",
    });
    let stage_runner = runner(client.clone());

    let output = stage_runner
        .run_stage(request(json!({
            "target_repository_root": "/tmp/target-repo",
        })))
        .await
        .expect("transient provider decode failures should retry and recover");

    assert_eq!(output.body, "status: completed");
    assert_eq!(client.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn workflow_live_does_not_retry_permission_errors() {
    let client = Arc::new(FlakyAgentClient {
        calls: AtomicUsize::new(0),
        first_error: "bypassPermissions requires --allow-dangerously-skip-permissions flag",
    });
    let stage_runner = runner(client.clone());

    let err = stage_runner
        .run_stage(request(json!({})))
        .await
        .expect_err("permission/config failures are not transport transients");

    assert!(
        err.to_string()
            .contains("bypassPermissions requires --allow-dangerously-skip-permissions")
    );
    assert_eq!(client.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn transient_classifier_matches_provider_decode_but_not_permission_errors() {
    assert!(transient_live_agent_error(
        "HTTP error: http_error: HTTP error: error decoding response body"
    ));
    assert!(!transient_live_agent_error(
        "bypassPermissions requires --allow-dangerously-skip-permissions flag"
    ));
}
