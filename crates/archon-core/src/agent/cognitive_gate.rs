use archon_cognitive::{
    ClassifyInput, CognitiveDecision, CognitiveSurface, ExecutiveAdvisoryInput,
    SituationClassifier, ToolGateInput, ToolUseGate, ToolVerdict, WorldModelState,
    direct_response_for, plan_runtime_advisory,
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
        self.record_cognitive_situation(&situation);
        self.current_situation = Some(situation);
    }

    pub(super) async fn run_cognitive_executive_advisory(&mut self) {
        self.cognitive_executive_reminder = None;
        let (Some(config), Some(policy), Some(ledger_dir), Some(situation)) = (
            self.cognitive_config.clone(),
            self.cognitive_policy.clone(),
            self.cognitive_ledger_dir.as_ref(),
            self.current_situation.clone(),
        ) else {
            return;
        };
        let ledger_dir = ledger_dir.clone();
        let outcome = match plan_runtime_advisory(
            &config,
            policy,
            ExecutiveAdvisoryInput {
                situation,
                working_dir: self.config.working_dir.clone(),
                world_model_state: WorldModelState::default(),
            },
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!(%error, "cognitive executive advisory planning failed");
                return;
            }
        };
        self.cognitive_executive_reminder = outcome.decision.as_ref().map(|decision| {
            format!(
                "Executive advisory: {}. Verification contract: {}. This is planning guidance only; execute through normal tool and permission gates.",
                decision.user_visible_summary,
                decision.verification_contract.as_deref().unwrap_or("none")
            )
        });
        tracing::debug!(
            stage = %outcome.snapshot.stage,
            selected_action = ?outcome.snapshot.selected_action,
            "cognitive executive advisory recorded"
        );
        if let Some(decision) = outcome.decision {
            archon_observability::spawn_blocking_named(
                "persist-cognitive-executive-advisory",
                move || match archon_cognitive::PersistentCognitiveStore::open(&ledger_dir) {
                    Ok(store) => {
                        let path = ledger_dir.join("cognitive-decisions.jsonl");
                        if let Err(error) = archon_cognitive::DecisionStore::new(store.db(), path)
                            .and_then(|decision_store| decision_store.record(&decision))
                        {
                            tracing::warn!(%error, "cognitive executive decision persistence failed");
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "cognitive executive persistence store unavailable");
                    }
                },
            );
        }
    }

    pub(super) async fn try_complete_trivial_cognitive_turn(&mut self) -> Option<String> {
        let situation = self.current_situation.as_ref()?;
        let response = direct_response_for(situation.kind)?;
        if !self.buffers_finalization_text() {
            self.send_event(AgentEvent::TextDelta(response.to_owned()))
                .await;
        }
        Some(response.to_owned())
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
            self.record_cognitive_decision(&decision);
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
            transcript_summary: None,
        })
        .await;
        self.state
            .add_tool_result(&tool.id, &result.content, result.is_error);
    }

    fn record_cognitive_situation(&self, situation: &archon_cognitive::Situation) {
        let Some(store) = self.cognitive_store.as_ref() else {
            return;
        };
        let result = store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .put_situation(situation);
        if let Err(err) = result {
            tracing::warn!(error = %err, "cognitive situation persistence failed");
        }
    }

    fn record_cognitive_decision(&self, decision: &CognitiveDecision) {
        let Some(store) = self.cognitive_store.as_ref() else {
            return;
        };
        let result = store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .put_decision(decision);
        if let Err(err) = result {
            tracing::warn!(error = %err, "cognitive decision persistence failed");
        }
    }
}
