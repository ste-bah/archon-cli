//! v0.1.12: AgentTool::execute reentrancy proof via barrier rendezvous.
//!
//! If AgentTool serializes invocations, the second call can't enter
//! run_to_completion before the first returns, so the barrier never
//! reaches 2 and the test hangs (caught by test runner timeout).
//! If concurrent, both proceed past the barrier and return quickly.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use archon_tools::agent_tool::{AgentTool, SubagentRequest};
use archon_tools::background_agents::BACKGROUND_AGENTS;
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
    install_subagent_executor,
};
use archon_tools::tool::{Tool, ToolContext};
use tokio::sync::Barrier;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Barrier-based mock executor — rendezvous proves concurrency.
// ---------------------------------------------------------------------------

struct BarrierExecutor {
    barrier: Barrier,
    mid_run_count: Mutex<usize>,
}

impl BarrierExecutor {
    fn new(parties: usize) -> Self {
        Self {
            barrier: Barrier::new(parties),
            mid_run_count: Mutex::new(0),
        }
    }

    fn mid_run_count(&self) -> usize {
        *self.mid_run_count.lock().unwrap()
    }
}

#[async_trait::async_trait]
impl SubagentExecutor for BarrierExecutor {
    async fn run_to_completion(
        &self,
        _subagent_id: String,
        request: SubagentRequest,
        _ctx: ToolContext,
        _cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        // Rendezvous: both invocations must reach here before either proceeds.
        self.barrier.wait().await;

        // Snapshot: BACKGROUND_AGENTS must contain both subagents simultaneously.
        let n = BACKGROUND_AGENTS.iter_running().len();
        let mut count = self.mid_run_count.lock().unwrap();
        *count = n.max(*count);

        Ok(format!("done:{}", request.prompt))
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
        0 // no auto-background
    }

    fn classify(&self, _request: &SubagentRequest) -> SubagentClassification {
        SubagentClassification::Foreground
    }
}

// ---------------------------------------------------------------------------
// Helper: build a ToolContext for test.
// ---------------------------------------------------------------------------

fn make_test_ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::current_dir().unwrap_or_default(),
        session_id: "test-barrier".into(),
        mode: archon_tools::tool::AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// The test.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_tool_execute_is_reentrant_under_parallel_invocation() {
    let executor = Arc::new(BarrierExecutor::new(2));
    install_subagent_executor(executor.clone());

    let tool = AgentTool::new();
    let ctx_a = make_test_ctx();
    let ctx_b = make_test_ctx();

    let start = std::time::Instant::now();
    let (a, b) = tokio::join!(
        tool.execute(
            serde_json::json!({"prompt": "A", "max_turns": 1, "timeout_secs": 30}),
            &ctx_a
        ),
        tool.execute(
            serde_json::json!({"prompt": "B", "max_turns": 1, "timeout_secs": 30}),
            &ctx_b
        ),
    );
    let elapsed = start.elapsed();

    // Both invocations succeeded.
    assert!(!a.is_error, "A failed: {}", a.content);
    assert!(!b.is_error, "B failed: {}", b.content);
    assert!(
        a.content.contains("done:A"),
        "unexpected A output: {}",
        a.content
    );
    assert!(
        b.content.contains("done:B"),
        "unexpected B output: {}",
        b.content
    );

    // Deterministic concurrency proof: both were registered at the barrier.
    assert_eq!(
        executor.mid_run_count(),
        2,
        "BACKGROUND_AGENTS must contain both subagents simultaneously at the barrier rendezvous"
    );

    // Secondary: wall-clock < 1s (barrier rendezvous is near-instant).
    assert!(
        elapsed < Duration::from_secs(1),
        "barrier test took {:?}, expected < 1s",
        elapsed
    );
}
