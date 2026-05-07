use super::*;

impl Agent {
    pub(super) async fn build_tool_context(
        &self,
        effective_mode: AgentMode,
        active_model: &str,
    ) -> ToolContext {
        let extra = self.config.extra_dirs.lock().await.clone();
        // TASK-AGS-105: compute in_fork once per turn from the
        // parent's message history so the SubagentExecutor can
        // enforce the fork-in-fork guard without crossing the
        // `state.messages` boundary into archon-tools.
        let in_fork = crate::agents::built_in::is_in_fork_child_by_messages(&self.state.messages);
        let ctx = ToolContext {
            working_dir: self.config.working_dir.clone(),
            session_id: self.config.session_id.clone(),
            mode: effective_mode,
            extra_dirs: extra,
            in_fork,
            // `nested` stays false here — only TaskCreateTool::execute
            // flips it to true when routing a subagent request through
            // the executor.
            nested: false,
            // TASK-AGS-107: propagate cancel token so subagent spawns
            // create child_token() chains for Ctrl+C cascading.
            cancel_parent: self.config.cancel_token.clone(),
            // GHOST-006: sandbox backend from session boot, checked at
            // both dispatch sites.
            sandbox: self.config.sandbox.clone(),
            activity_sink: self.provider_model_activity_sink(active_model),
        };
        ctx
    }
}
