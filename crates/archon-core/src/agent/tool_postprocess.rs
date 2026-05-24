use super::tool_postprocess_steps::PostprocessFlow;
use super::tool_types::PreflightResult;
use super::*;

impl Agent {
    pub(super) async fn postprocess_tools(
        &mut self,
        allowed: &[PreflightResult],
        dispatch_results: Vec<ToolResult>,
        ctx: &ToolContext,
        active_model: &str,
    ) -> Option<String> {
        let mut flow = PostprocessFlow::default();
        for (pre, result) in allowed.iter().zip(dispatch_results.into_iter()) {
            self.postprocess_single_tool(pre, result, ctx, active_model, &mut flow)
                .await;
        }

        self.fill_orphan_tool_results(allowed);

        flow.prevent_continuation_reason
    }

    fn fill_orphan_tool_results(&mut self, allowed: &[PreflightResult]) {
        let expected_ids: Vec<String> = allowed.iter().map(|p| p.tool_id.clone()).collect();
        for id in self.state.fill_missing_tool_results(&expected_ids) {
            tracing::warn!(
                tool_use_id = %id,
                "tool dispatch did not produce a result; filled with synthetic error"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_orphan_tool_results_appends_synthetic_for_missing_ids() {
        let mut state = ConversationState::default();
        state.add_assistant_message(vec![
            serde_json::json!({"type": "tool_use", "id": "tool-1", "name": "Read", "input": {}}),
            serde_json::json!({"type": "tool_use", "id": "tool-2", "name": "Write", "input": {}}),
        ]);
        state.add_tool_result("tool-1", "ok", false);

        let missing =
            state.fill_missing_tool_results(&["tool-1".to_string(), "tool-2".to_string()]);

        assert_eq!(missing, vec!["tool-2".to_string()]);
        let blocks = state.messages[1]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1]["tool_use_id"], "tool-2");
        assert_eq!(blocks[1]["is_error"], true);
    }

    #[test]
    fn fill_orphan_tool_results_idempotent_when_all_present() {
        let mut state = ConversationState::default();
        state.add_tool_result("tool-1", "ok", false);

        let missing = state.fill_missing_tool_results(&["tool-1".to_string()]);

        assert!(missing.is_empty());
        assert_eq!(state.messages[0]["content"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn fill_orphan_tool_results_no_op_when_allowed_empty() {
        let mut state = ConversationState::default();

        let missing = state.fill_missing_tool_results(&[]);

        assert!(missing.is_empty());
        assert!(state.messages.is_empty());
    }

    #[test]
    fn context_tool_output_cap_preserves_ui_result_shape() {
        let content = "x".repeat(80_000);
        let capped =
            crate::agent::tool_result_context::cap_tool_output_for_context("Agent", &content);

        assert!(capped.truncated);
        assert!(capped.content.contains("tool output trimmed"));
        assert!(capped.stored_chars <= capped.limit_chars);
    }
}
