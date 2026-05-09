use super::*;

impl Agent {
    pub(super) async fn fire_before_agent_run_hook(&self, user_input: &str) {
        self.fire_runtime_hook(
            crate::hooks::HookEvent::BeforeAgentRun,
            serde_json::json!({
                "hook_event": "BeforeAgentRun",
                "stage": "before_agent_run",
                "turn": self.turn_number,
                "input_chars": user_input.chars().count(),
            }),
        )
        .await;
    }

    pub(super) async fn fire_after_agent_run_hook(&self, status: &str, error: Option<String>) {
        self.fire_runtime_hook(
            crate::hooks::HookEvent::AfterAgentRun,
            serde_json::json!({
                "hook_event": "AfterAgentRun",
                "stage": "after_agent_run",
                "turn": self.turn_number,
                "status": status,
                "error": error,
            }),
        )
        .await;
    }

    pub(super) async fn fire_before_prompt_build_hook(&self, iteration: u32) {
        self.fire_runtime_hook(
            crate::hooks::HookEvent::BeforePromptBuild,
            serde_json::json!({
                "hook_event": "BeforePromptBuild",
                "stage": "before_prompt_build",
                "turn": self.turn_number,
                "iteration": iteration,
                "message_count": self.state.messages.len(),
            }),
        )
        .await;
    }

    pub(super) async fn fire_after_prompt_build_hook(&self, request: &LlmRequest, iteration: u32) {
        self.fire_runtime_hook(
            crate::hooks::HookEvent::AfterPromptBuild,
            serde_json::json!({
                "hook_event": "AfterPromptBuild",
                "stage": "after_prompt_build",
                "turn": self.turn_number,
                "iteration": iteration,
                "model": request.model,
                "system_blocks": request.system.len(),
                "message_count": request.messages.len(),
                "tool_count": request.tools.len(),
            }),
        )
        .await;
    }

    pub(super) async fn fire_before_tool_call_hook(
        &self,
        tool_name: &str,
        tool_id: &str,
        input: &serde_json::Value,
    ) {
        self.fire_runtime_hook(
            crate::hooks::HookEvent::BeforeToolCall,
            serde_json::json!({
                "hook_event": "BeforeToolCall",
                "stage": "before_tool_call",
                "tool_name": tool_name,
                "tool_id": tool_id,
                "tool_input": input,
            }),
        )
        .await;
    }

    pub(super) async fn fire_after_tool_call_hook(
        &self,
        tool_name: &str,
        tool_id: &str,
        result: &ToolResult,
    ) {
        self.fire_runtime_hook(
            crate::hooks::HookEvent::AfterToolCall,
            serde_json::json!({
                "hook_event": "AfterToolCall",
                "stage": "after_tool_call",
                "tool_name": tool_name,
                "tool_id": tool_id,
                "is_error": result.is_error,
                "output_chars": result.content.chars().count(),
            }),
        )
        .await;
    }

    pub(super) async fn fire_before_learning_event_hook(
        &self,
        event_type: &str,
        payload: &UserCorrectionEventPayload,
    ) {
        self.fire_runtime_hook(
            crate::hooks::HookEvent::BeforeLearningEvent,
            learning_event_payload(
                "BeforeLearningEvent",
                "before_learning_event",
                event_type,
                payload,
            ),
        )
        .await;
    }

    pub(super) async fn fire_after_learning_event_hook(
        &self,
        event_type: &str,
        payload: &UserCorrectionEventPayload,
    ) {
        self.fire_runtime_hook(
            crate::hooks::HookEvent::AfterLearningEvent,
            learning_event_payload(
                "AfterLearningEvent",
                "after_learning_event",
                event_type,
                payload,
            ),
        )
        .await;
    }

    async fn fire_runtime_hook(&self, event: crate::hooks::HookEvent, payload: serde_json::Value) {
        let result = self.fire_hook(event.clone(), payload).await;
        trace_ignored_runtime_hook_output(&event, &result);
    }
}

fn learning_event_payload(
    hook_event: &str,
    stage: &str,
    event_type: &str,
    payload: &UserCorrectionEventPayload,
) -> serde_json::Value {
    serde_json::json!({
        "hook_event": hook_event,
        "stage": stage,
        "learning_event_type": event_type,
        "correction_type": payload.correction_type,
        "top_rule_id": payload.top_rule_id,
        "user_input_excerpt": payload.user_input_excerpt,
        "session_context": payload.session_context,
    })
}

fn trace_ignored_runtime_hook_output(
    event: &crate::hooks::HookEvent,
    result: &crate::hooks::AggregatedHookResult,
) {
    if result.is_blocked()
        || result.updated_input.is_some()
        || result.updated_mcp_tool_output.is_some()
        || !result.updated_permissions.is_empty()
        || result.prevent_continuation
        || result.retry
    {
        tracing::warn!(
            hook_event = %event,
            "runtime lifecycle hook returned behaviour-changing output; ignored"
        );
    }
}
