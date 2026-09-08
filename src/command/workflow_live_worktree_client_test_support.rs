use super::*;

#[async_trait::async_trait]
impl WorkflowLlmClient for GeneratedV2WorktreeRunClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.planner_calls.fetch_add(1, Ordering::SeqCst);
        Ok(WorkflowAgentOutcome {
            content: r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "planner", task: "Return typed implementation inventory." });
  const implemented = await w.fanout("implementation", inventory.items, { role: "coder", itemKind: "implementation", targetFilesFromItem: true, write: "worktree", task: "Edit the assigned target file in the current repository root." });
  await w.finalReport("final", { inputs: [inventory, implemented], task: "Produce final report from typed evidence." });
}