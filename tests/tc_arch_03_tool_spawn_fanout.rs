//! TC-ARCH-03 (REQ-FOR-D2): Tool layer spawns 100 subagents from one parent.
//!
//! Invoke AgentTool::execute 100 times with run_in_background=true.
//! Assert:
//! - Median execute time is <10ms and max is below the child-run delay
//! - All 100 agent_ids are unique
//! - BACKGROUND_AGENTS has 100 entries after all spawns

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use archon_tools::agent_tool::{AgentTool, SubagentRequest};
use archon_tools::background_agents::BACKGROUND_AGENTS;
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
};
use archon_tools::tool::{Tool, ToolContext};

/// Delayed executor that makes waiting for a child run observable without
/// making a real LLM call.
struct DelayedExecutor;

#[async_trait]
impl SubagentExecutor for DelayedExecutor {
    async fn run_to_completion(
        &self,
        _subagent_id: String,
        _req: SubagentRequest,
        _ctx: ToolContext,
        _cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        Ok("instant".to_string())
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spawn_100_subagents_with_unique_ids() {
    // Install the delayed executor (OnceLock: first wins). If execute waits
    // for the child run, its 250ms delay makes that visible in the timing.
    archon_tools::subagent_executor::install_subagent_executor(Arc::new(DelayedExecutor));

    // Reap any leftover entries from other tests
    let _ = BACKGROUND_AGENTS.reap_finished();

    let tool = AgentTool::new();
    let ctx = ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "tc-arch-03".into(),
        ..Default::default()
    };

    // CI runners can pay one-time runtime/registry warm-up cost on the first
    // call. TC-ARCH-03 measures the hot spawn path, so warm it before timing.
    let warmup = tool
        .execute(
            json!({
                "prompt": "subagent warmup",
                "run_in_background": true,
            }),
            &ctx,
        )
        .await;
    assert!(
        !warmup.is_error,
        "warmup execute returned error: {}",
        warmup.content
    );
    let _ = BACKGROUND_AGENTS.reap_finished();

    let count = 100usize;
    let mut agent_ids = HashSet::new();
    let mut timings = Vec::with_capacity(count);

    for i in 0..count {
        let input = json!({
            "prompt": format!("subagent task {i}"),
            "run_in_background": true,
        });

        let start = Instant::now();
        let result = tool.execute(input, &ctx).await;
        let elapsed = start.elapsed();

        assert!(
            !result.is_error,
            "execute {i} returned error: {}",
            result.content
        );

        // Extract agent_id from the JSON response
        let parsed: serde_json::Value =
            serde_json::from_str(&result.content).expect("result not valid JSON");
        let id = parsed["agent_id"]
            .as_str()
            .expect("no agent_id in result")
            .to_string();
        agent_ids.insert(id);

        timings.push((i, elapsed));
    }

    // All 100 IDs unique
    assert_eq!(
        agent_ids.len(),
        count,
        "TC-ARCH-03: expected {count} unique agent_ids, got {}",
        agent_ids.len()
    );

    let mut sorted_elapsed: Vec<_> = timings.iter().map(|(_, elapsed)| *elapsed).collect();
    sorted_elapsed.sort_unstable();
    let median = (sorted_elapsed[count / 2 - 1] + sorted_elapsed[count / 2]) / 2;
    let max = sorted_elapsed[count - 1];
    let slow_calls: Vec<_> = timings
        .iter()
        .filter(|(_, elapsed)| elapsed.as_millis() >= 10)
        .copied()
        .collect();

    // A median gate tolerates isolated CI scheduling stalls. The max must
    // remain below 200ms, safely under the 250ms a blocking child wait costs.
    assert!(
        median.as_millis() < 10 && max.as_millis() < 200,
        "TC-ARCH-03: median execute time was {median:?}, max was {max:?}; slow calls (>=10ms): {:?}",
        &slow_calls[..slow_calls.len().min(10)]
    );

    // Registry has at least count entries (may include leftovers from
    // other tests if they share the process, but never fewer)
    let running = BACKGROUND_AGENTS.iter_running().len();
    let finished = BACKGROUND_AGENTS.reap_finished().len();
    let all = running + finished;
    // At minimum, 100 were registered this test
    assert!(
        all >= count,
        "TC-ARCH-03: registry should have at least {count} entries, found {all}"
    );
}
