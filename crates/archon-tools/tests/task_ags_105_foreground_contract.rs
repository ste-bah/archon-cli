//! TASK-AGS-105: foreground contract regression guard.
//!
//! This test is the unit-layer guard that would have caught the
//! TASK-AGS-104 silent break. It installs a FixedStringSubagentExecutor
//! that returns a fixed string from `run_to_completion`, calls
//! `AgentTool::execute` with the default (foreground) path, and asserts
//! the returned `ToolResult.content` is the real subagent text — NOT a
//! spawn marker JSON.
//!
//! This test MUST live in its own test binary (own file under `tests/`)
//! because OnceLock permits install-once-per-process; the other 104
//! tests install NoopSubagentExecutor and cannot coexist with a
//! FixedString variant in the same process.

use std::sync::Arc;

use serde_json::json;

use archon_tools::agent_tool::{AgentTool, SubagentRequest};
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
    install_subagent_executor,
};
use archon_tools::tool::{Tool, ToolContext};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

struct FixedStringExecutor(&'static str);

#[async_trait]
impl SubagentExecutor for FixedStringExecutor {
    async fn run_to_completion(
        &self,
        _subagent_id: String,
        _req: SubagentRequest,
        _ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        tokio::select! {
            _ = cancel.cancelled() => Err(ExecutorError::Internal("cancelled".into())),
            _ = std::future::ready(()) => Ok(self.0.to_string()),
        }
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

fn install_test_executor_returning(s: &'static str) {
    install_subagent_executor(Arc::new(FixedStringExecutor(s)));
}

fn make_ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "task-ags-105-fg".into(),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreground_execute_returns_real_text() {
    install_test_executor_returning("real subagent text");
    let tool = AgentTool::new();
    let input = json!({ "prompt": "Do something", "run_in_background": false });
    let result = tool.execute(input, &make_ctx()).await;
    assert!(!result.is_error, "unexpected error: {}", result.content);
    assert_eq!(
        result.content, "real subagent text",
        "foreground execute must return the real subagent text, not a spawn marker JSON"
    );
    // Guard that we did NOT receive a JSON spawn marker:
    assert!(
        serde_json::from_str::<serde_json::Value>(&result.content)
            .ok()
            .and_then(|v| v.get("agent_id").cloned())
            .is_none(),
        "foreground result must not contain an agent_id field"
    );
}
