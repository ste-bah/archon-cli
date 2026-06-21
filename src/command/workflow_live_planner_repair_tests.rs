use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use archon_pipeline::runner::{LlmClient, LlmResponse};
use archon_workflow::WorkflowStore;

use super::plan_live;

struct TwoStepRepairPlanner {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl LlmClient for TwoStepRepairPlanner {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LlmResponse {
            content: response_for_call(call).to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

fn response_for_call(call: usize) -> &'static str {
    match call {
        0 => {
            r#"
export default async function workflow(w) {
  await w.shell("bad", {});
}
"#
        }
        1 => {
            r#"
export default async function workflow(w) {
  await w.agent("bad", { model: "claude-opus-4-8", task: "inspect" });
}
"#
        }
        _ => {
            r#"
export default async function workflow(w) {
  await w.agent("discover", { role: "researcher", task: "inspect" });
}
"#
        }
    }
}

#[tokio::test]
async fn live_planner_uses_bounded_iterative_harness_repair() {
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(16);
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let planner = Arc::new(TwoStepRepairPlanner {
        calls: AtomicUsize::new(0),
    });

    let plan = plan_live(&store, "inspect the repository", planner.clone(), tui_tx)
        .await
        .expect("second repaired harness should validate");

    assert_eq!(plan.calls.len(), 1);
    assert_eq!(plan.calls[0].id, "discover");
    assert_eq!(planner.calls.load(Ordering::SeqCst), 3);
}
