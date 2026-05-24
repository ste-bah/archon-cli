use super::*;

impl AgentSubagentExecutor {
    pub(super) async fn run_subagent_to_completion(
        &self,
        subagent_id: String,
        request: SubagentRequest,
        ctx: ToolContext,
        _cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        let ids = self.register_subagent_run(&subagent_id, &request).await?;
        self.fire_subagent_start_hooks(&ids.manager_id, &request, ctx.nested)
            .await;
        let prepared = self
            .prepare_subagent_run(&ids.manager_id, &request, &ctx)
            .await?;
        let runner = self
            .build_subagent_runner(&ids, &request, &ctx, &prepared)
            .await?;
        let activity_model = runner.model().to_string();

        self.emit_subagent_started(
            &ids.cache_id,
            &prepared.activity_agent_type,
            &activity_model,
        );
        let runner_result = runner.run(&request.prompt).await;
        let inner_result = runner_result.map_err(|e| format!("Subagent failed: {e}"));
        self.emit_subagent_finished(
            &ids.cache_id,
            &prepared.activity_agent_type,
            &activity_model,
            &inner_result,
        );
        self.on_inner_complete(ids.cache_id, inner_result.clone())
            .await;

        inner_result.map_err(ExecutorError::Internal)
    }
}
