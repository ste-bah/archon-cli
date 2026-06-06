use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use archon_pipeline::runner::{LlmClient, LlmResponse};
use serde_json::json;

use super::{
    ProviderTier, StageKind, StageRunRequest, allowed_tools, extract_yaml, plan_live,
    request_target_repository_root,
};

struct InvalidPlanner;

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
