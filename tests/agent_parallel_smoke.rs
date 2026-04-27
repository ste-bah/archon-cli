//! v0.1.12: End-to-end parallel agent dispatch smoke test.
//!
//! Simulates 3 concurrent Agent tool calls (what happens when the LLM
//! emits 3 Agent tool_use blocks in one turn after deliverable B).
//! Asserts all 3 complete, wall-clock confirms parallelism, and
//! results are delivered with correct payloads.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use archon_tools::agent_tool::{AgentTool, SubagentRequest};
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
};
use archon_tools::tool::{Tool, ToolContext};

// ---------------------------------------------------------------------------
// Mock executor — delays 500ms then returns the prompt as output.
// ---------------------------------------------------------------------------

struct DelayingExecutor {
    delay_ms: u64,
}

#[async_trait]
impl SubagentExecutor for DelayingExecutor {
    async fn run_to_completion(
        &self,
        _subagent_id: String,
        request: SubagentRequest,
        _ctx: ToolContext,
        _cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        Ok(format!("result:{}", request.prompt))
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

    fn classify(&self, _request: &SubagentRequest) -> SubagentClassification {
        SubagentClassification::Foreground
    }
}

fn make_ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::current_dir().unwrap_or_default(),
        session_id: "test-e2e-parallel".into(),
        mode: archon_tools::tool::AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Test: 3 concurrent Agent tool calls — like the LLM emitting 3 Agent
// tool_use blocks in one turn after deliverable B.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_concurrent_agent_tool_calls_complete_in_parallel() {
    // Install the executor before AgentTool::execute resolves it.
    archon_tools::subagent_executor::install_subagent_executor(Arc::new(DelayingExecutor {
        delay_ms: 500,
    }));

    let tool = AgentTool::new();
    let ctx_a = make_ctx();
    let ctx_b = make_ctx();
    let ctx_c = make_ctx();

    let start = std::time::Instant::now();
    let (a, b, c) = tokio::join!(
        tool.execute(
            json!({"prompt": "apple", "max_turns": 1, "timeout_secs": 30}),
            &ctx_a
        ),
        tool.execute(
            json!({"prompt": "banana", "max_turns": 1, "timeout_secs": 30}),
            &ctx_b
        ),
        tool.execute(
            json!({"prompt": "cherry", "max_turns": 1, "timeout_secs": 30}),
            &ctx_c
        ),
    );
    let elapsed = start.elapsed();

    // All 3 completed successfully.
    assert!(!a.is_error, "apple failed: {}", a.content);
    assert!(!b.is_error, "banana failed: {}", b.content);
    assert!(!c.is_error, "cherry failed: {}", c.content);

    assert!(
        a.content.contains("result:apple"),
        "unexpected: {}",
        a.content
    );
    assert!(
        b.content.contains("result:banana"),
        "unexpected: {}",
        b.content
    );
    assert!(
        c.content.contains("result:cherry"),
        "unexpected: {}",
        c.content
    );

    // Concurrent: 3 × 500ms serial = 1500ms. Parallel should be ~500ms.
    // Allow 1.5× headroom for CI variance.
    assert!(
        elapsed < Duration::from_millis(1000),
        "3 concurrent agents took {:?}, expected < 1000ms (serial would be ~1500ms)",
        elapsed
    );
}
