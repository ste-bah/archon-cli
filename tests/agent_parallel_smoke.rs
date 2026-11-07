//! v0.1.12: End-to-end parallel agent dispatch smoke test.
//!
//! Simulates 3 concurrent Agent tool calls (what happens when the LLM
//! emits 3 Agent tool_use blocks in one turn after deliverable B).
//! Asserts all 3 complete, a barrier proves their execution overlaps, and
//! results are delivered with correct payloads.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::Barrier;
use tokio_util::sync::CancellationToken;

use archon_tools::agent_tool::{AgentTool, SubagentRequest};
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
};
use archon_tools::tool::{Tool, ToolContext};

// ---------------------------------------------------------------------------
// Mock executor — all three calls must arrive before any can return.
// ---------------------------------------------------------------------------

struct BarrierExecutor {
    barrier: Barrier,
    in_flight: AtomicUsize,
    peak_in_flight: AtomicUsize,
}

#[async_trait]
impl SubagentExecutor for BarrierExecutor {
    async fn run_to_completion(
        &self,
        _subagent_id: String,
        request: SubagentRequest,
        _ctx: ToolContext,
        _cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        let in_flight = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak_in_flight.fetch_max(in_flight, Ordering::SeqCst);

        // A serial implementation cannot release the barrier. The timeout is
        // only a hang guard; concurrency is proved by the rendezvous and peak.
        tokio::time::timeout(Duration::from_secs(10), self.barrier.wait())
            .await
            .expect("all three agent calls must reach the barrier; execution may be serialized");

        self.in_flight.fetch_sub(1, Ordering::SeqCst);
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
    let executor = Arc::new(BarrierExecutor {
        barrier: Barrier::new(3),
        in_flight: AtomicUsize::new(0),
        peak_in_flight: AtomicUsize::new(0),
    });
    archon_tools::subagent_executor::install_subagent_executor(executor.clone());

    let tool = AgentTool::new();
    let ctx_a = make_ctx();
    let ctx_b = make_ctx();
    let ctx_c = make_ctx();

    let (a, b, c) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(
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
        )
    })
    .await
    .expect("three concurrent AgentTool calls must complete without hanging");

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

    assert_eq!(
        executor.peak_in_flight.load(Ordering::SeqCst),
        3,
        "all three agent calls must be in flight together at the barrier"
    );
}
