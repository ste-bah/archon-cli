use archon_cognitive::{
    ClassifyInput, CognitiveDecision, CognitiveSurface, SituationClassifier, ToolGateInput,
    ToolUseGate, ToolVerdict, direct_response_for,
};

use super::*;

impl Agent {
    pub(super) fn classify_cognitive_situation(&mut self, user_input: &str) {
        let situation = SituationClassifier.classify(ClassifyInput {
            user_text: user_input,
            session_id: &self.config.session_id,
            turn_number: self.turn_number,
            surface: CognitiveSurface::Tui,
        });
        tracing::debug!(
            kind = situation.kind.as_str(),
            confidence = situation.confidence_score,
            reason = %situation.reason,
            "classified cognitive situation"
        );
        self.current_situation = Some(situation);
    }

    pub(super) async fn try_complete_trivial_cognitive_turn(&mut self, user_input: &str) -> bool {
        let Some(situation) = self.current_situation.as_ref() else {
            return false;
        };
        let Some(response) = direct_response_for(situation.kind) else {
            return false;
        };
        self.send_event(AgentEvent::TextDelta(response.to_owned()))
            .await;
        self.state.add_assistant_message(vec![serde_json::json!({
            "type": "text",
            "text": response,
        })]);
        let active_model = self.active_model().await;
        self.complete_turn_without_tools(user_input, 0, 0, 0, 0, &active_model)
            .await;
        true
    }

    pub(super) async fn cognitive_gate_allows_tool(
        &mut self,
        tool: &PendingToolCall,
        input: &serde_json::Value,
    ) -> bool {
        let Some(situation) = self.current_situation.as_ref() else {
            return true;
        };
        let verdict = ToolUseGate.evaluate(ToolGateInput {
            situation,
            tool_name: &tool.name,
            tool_input: input,
            working_dir: &self.config.working_dir,
        });
        if verdict.is_allow() {
            return true;
        }
        self.record_cognitive_tool_decision(tool, &verdict).await;
        false
    }

    async fn record_cognitive_tool_decision(
        &mut self,
        tool: &PendingToolCall,
        verdict: &ToolVerdict,
    ) {
        if let Some(situation) = self.current_situation.as_ref() {
            let decision = CognitiveDecision::for_tool(situation, &tool.name, verdict.clone());
            tracing::debug!(
                tool = %tool.name,
                situation = situation.kind.as_str(),
                reason = %decision.reason,
                "cognitive tool gate suppressed tool"
            );
        }
        let result = match verdict {
            ToolVerdict::Suppress { reason } => {
                ToolResult::success(format!("Tool suppressed by cognitive gate: {reason}"))
            }
            ToolVerdict::ConvertToContextNote { note } => ToolResult::success(note.clone()),
            ToolVerdict::Allow { .. } => return,
        };
        self.send_event(AgentEvent::ToolCallComplete {
            name: tool.name.clone(),
            id: tool.id.clone(),
            result: result.clone(),
        })
        .await;
        self.state
            .add_tool_result(&tool.id, &result.content, result.is_error);
    }
}
