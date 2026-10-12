use super::*;

impl AgentSubagentExecutor {
    pub(super) async fn run_subagent_to_completion_with_system_metadata(
        &self,
        subagent_id: String,
        request: SubagentRequest,
        system: Vec<serde_json::Value>,
        ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<archon_tools::subagent_executor::SubagentCompletion, ExecutorError> {
        let mut row = self.activity_row(&subagent_id);
        let result = self
            .run_subagent_in_slot(subagent_id, request, system, ctx, cancel.clone(), &mut row)
            .await;
        row.settle(
            &result
                .as_ref()
                .map(|completion| completion.content.clone())
                .map_err(Clone::clone),
            &cancel,
        );
        result
    }
}
