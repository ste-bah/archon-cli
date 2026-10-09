//! Tool lookup, audit and execution enter the captured host admission.
use super::*;
impl ToolRegistry {
    pub async fn dispatch(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> ToolResult {
        let work = self.dispatch_owned(tool_name, input, ctx);
        match ctx
            .run_store
            .as_ref()
            .and_then(|store| store.admission.as_ref())
        {
            // A control stop reaches here only without an enclosing fence;
            // its text names the control decision, never a tool failure.
            Some(fence) => fence
                .execute(work)
                .await
                .unwrap_or_else(|stop| ToolResult::refusal(stop.to_string())),
            None => work.await,
        }
    }
    /// Dispatch a tool call: check mode, check sandbox, execute, return result.
    async fn dispatch_owned(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> ToolResult {
        // Check if tool is allowed in current mode
        if !is_tool_allowed_in_mode(tool_name, ctx.mode) {
            // Append the intercepted call to the session-scoped immutable audit
            // log. The editable document remains separate and is only opened by
            // `/plan open`. IO failures are logged but MUST NOT replace the
            // block: returning an error so the model sees the tool failed is the
            // primary behaviour; the audit append is additive.
            match crate::plan_file::plan_audit_path(&ctx.working_dir, &ctx.session_id) {
                Ok(audit_path) => {
                    if let Err(error) =
                        crate::plan_file::append_plan_entry(&audit_path, tool_name, &input)
                    {
                        tracing::warn!(
                            error = %error,
                            audit_path = %audit_path.display(),
                            tool = tool_name,
                            "failed to append intercepted tool call to session audit log"
                        );
                    }
                }
                Err(error) => tracing::warn!(
                    error = %error,
                    session_id = %ctx.session_id,
                    tool = tool_name,
                    "refused unsafe session ID for Plan Mode audit log"
                ),
            }
            emit_tool_activity(
                ctx,
                tool_name,
                AgentActivityKind::ToolFailed,
                AgentActivityStatus::Failed,
            );
            return ToolResult::refusal(format!(
                "Tool '{tool_name}' is not available in Plan Mode. Plan Mode blocks working-tree mutations by default; only the canonical Plan-safe allowlist is available, including TaskCreate, TaskUpdate, and Agent. The call has been recorded in the session audit for review."
            ));
        }

        // Look up tool
        let tool = match self.get(tool_name) {
            Some(t) => t,
            None => {
                emit_tool_activity(
                    ctx,
                    tool_name,
                    AgentActivityKind::ToolFailed,
                    AgentActivityStatus::Failed,
                );
                return ToolResult::error(format!(
                    "Unknown tool: '{tool_name}'. Available tools: {}",
                    self.tool_names().join(", ")
                ));
            }
        };

        // Execute
        crate::tool_run_admission::execute_tool_attempt(tool, input, ctx, false).await
    }
}
