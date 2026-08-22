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

        ToolContext {
            working_dir: self.config.working_dir.clone(),
            session_id: self.config.session_id.clone(),
            // Top-level agent: no subagent is in scope. The executor stamps a
            // real id onto the child context it derives from this one.
            subagent_id: None,
            // `turn_number` is incremented once, in `begin_process_turn`, and
            // this is built per tool round *inside* the turn — so it names the
            // turn the tools are running in and changes when the turn does.
            // Qualified by the session because the counter restarts at 1 for
            // every agent.
            turn_id: Some(format!("{}#{}", self.config.session_id, self.turn_number())),
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
            // #201 Phase 1: same field the read-before-edit guard reads, so the
            // guard and the tools cannot disagree about which world they are
            // in. Phase 2 is where each backend populates it.
            fs: self.config.fs.clone(),
            activity_sink: self.provider_model_activity_sink(active_model),
            tool_run_parent_action_id: self.guardrail_action_id.clone(),
            tool_run_tool_use_id: None,
            tool_run_attempt: 0,
            tool_run_admission: self.tool_run_admission_callback.clone(),
            tool_run_outcome: self.tool_run_outcome_callback.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::test_agent;
    use archon_tools::tool::AgentMode;

    #[tokio::test]
    async fn top_level_tool_context_carries_no_subagent_id() {
        let agent = test_agent();

        let ctx = agent.build_tool_context(AgentMode::Normal, "mock").await;

        assert_eq!(ctx.subagent_id, None);
    }

    /// `sandbox.scope = "turn"` is only honest if this changes when the turn
    /// does. `tool_run_parent_action_id` is the field that looks like it would
    /// serve and does not: it is `None` on the plain interactive path, it is a
    /// guardrail *action* id that spans however many turns the action takes,
    /// and the world-model path assigns it the session id outright — so a
    /// `turn`-scoped sandbox keyed on it would hold one container for the whole
    /// session while reporting per-turn isolation.
    #[tokio::test]
    async fn the_turn_identity_changes_when_the_turn_does_and_the_action_id_does_not() {
        let mut agent = test_agent();
        agent.set_guardrail_action_id(Some("one-action-many-turns".into()));

        agent.begin_process_turn("first").await;
        let first = agent.build_tool_context(AgentMode::Normal, "mock").await;
        agent.begin_process_turn("second").await;
        let second = agent.build_tool_context(AgentMode::Normal, "mock").await;

        assert_ne!(
            first.turn_id, second.turn_id,
            "a turn identity that survives a turn boundary is a session id"
        );
        assert!(first.turn_id.is_some(), "the top-level agent has turns");
        assert_eq!(
            first.tool_run_parent_action_id, second.tool_run_parent_action_id,
            "the guardrail action id is stable across turns, which is exactly \
             why it cannot be the turn identity"
        );
    }
}
