use std::fs;
use std::sync::Arc;

use archon_tools::agent_tool::{AgentTool, SubagentRequest};
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
    install_subagent_executor,
};
use archon_tools::tool::{Tool, ToolContext};
use async_trait::async_trait;
use serde_json::json;
use tokio_util::sync::CancellationToken;

struct NoopExecutor;
struct MutatingExecutor;

#[async_trait]
impl SubagentExecutor for NoopExecutor {
    async fn run_to_completion(
        &self,
        _subagent_id: String,
        _request: SubagentRequest,
        _ctx: ToolContext,
        _cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        Ok("no changes made".into())
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

    fn classify(&self, request: &SubagentRequest) -> SubagentClassification {
        if request.run_in_background {
            SubagentClassification::ExplicitBackground
        } else {
            SubagentClassification::Foreground
        }
    }
}

#[async_trait]
impl SubagentExecutor for MutatingExecutor {
    async fn run_to_completion(
        &self,
        _subagent_id: String,
        _request: SubagentRequest,
        ctx: ToolContext,
        _cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        fs::write(ctx.working_dir.join("target.md"), "changed")
            .map_err(|err| ExecutorError::Internal(err.to_string()))?;
        Ok("changed target".into())
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

    fn classify(&self, request: &SubagentRequest) -> SubagentClassification {
        if request.run_in_background {
            SubagentClassification::ExplicitBackground
        } else {
            SubagentClassification::Foreground
        }
    }
}

fn ctx(dir: &tempfile::TempDir) -> ToolContext {
    ToolContext {
        working_dir: dir.path().to_path_buf(),
        session_id: "agent-expected-mutation-test".into(),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn expected_target_files_fail_when_subagent_changes_nothing() {
    install_subagent_executor(Arc::new(NoopExecutor));
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("target.md"), "original").unwrap();

    let result = AgentTool::new()
        .execute(
            json!({
                "prompt": "edit target.md",
                "expected_target_files": ["target.md"]
            }),
            &ctx(&dir),
        )
        .await;

    assert!(result.is_error, "expected mutation guard must fail");
    assert!(result.content.contains("expected target file"));
    assert_eq!(
        fs::read_to_string(dir.path().join("target.md")).unwrap(),
        "original"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn expected_target_files_pass_when_subagent_changes_file() {
    install_subagent_executor(Arc::new(MutatingExecutor));
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("target.md"), "original").unwrap();

    let result = AgentTool::new()
        .execute(
            json!({
                "prompt": "edit target.md",
                "expected_target_files": ["target.md"]
            }),
            &ctx(&dir),
        )
        .await;

    assert!(!result.is_error, "unexpected error: {}", result.content);
    assert_eq!(result.content, "changed target");
    assert_eq!(
        fs::read_to_string(dir.path().join("target.md")).unwrap(),
        "changed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn expected_target_files_reject_background_subagents() {
    install_subagent_executor(Arc::new(NoopExecutor));
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("target.md"), "original").unwrap();

    let result = AgentTool::new()
        .execute(
            json!({
                "prompt": "edit target.md",
                "run_in_background": true,
                "expected_target_files": ["target.md"]
            }),
            &ctx(&dir),
        )
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("foreground subagents"));
}
