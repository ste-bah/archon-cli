use std::time::Duration;

use archon_cognitive::{
    ClassifyInput, CognitiveDecision, CognitiveSurface, ExecutiveAdvisoryInput, LiveTurnOutcome,
    ReflectionWriter, ShadowComparison, ShadowTurnInput, ShadowTurnObserver, SituationClassifier,
    ToolGateInput, ToolUseGate, ToolVerdict, TriggeredReflectInput, TurnSignals, WorldModelScorer,
    direct_response_for, observed_action_from_tools, plan_runtime_advisory,
    plan_runtime_advisory_with,
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
        let input = ExecutiveAdvisoryInput {
            situation,
            working_dir: self.config.working_dir.clone(),
            world_model_state: self.cognitive_world_model_state.clone(),
        };
        // With a backend injected the scorer may consult model predictions;
        // whether it actually does is decided by `world_model_state`, so a
        // shadow-only state still takes the heuristic path. Without a backend
        // there is nothing to consult and we skip the generic entirely.
        let planned = match self.cognitive_prediction_backend.clone() {
            Some(backend) => plan_runtime_advisory_with(
                &config,
                policy,
                input,
                &WorldModelScorer::new(backend, true, true),
            ),
            None => plan_runtime_advisory(&config, policy, input),
        };
        let outcome = match planned {
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

    /// Run the executive loop over this turn as a non-executing observer.
    ///
    /// This is the live call site for `ExecutiveLoop` (issue #76). It runs
    /// before the LLM request so the shadow plan is persisted *before* the live
    /// turn executes, which is what makes the later comparison a prediction
    /// rather than a rationalisation.
    ///
    /// Observationally inert by construction: it takes no action, writes only
    /// to the shadow relation and its own ledger, and is bounded by the
    /// configured pipeline budget. If it is slow, panics, or fails, the turn
    /// proceeds exactly as it would have without it.
    pub(super) async fn run_cognitive_shadow_observation(&mut self, user_input: &str) {
        let (Some(config), Some(policy), Some(ledger_dir)) = (
            self.cognitive_config.clone(),
            self.cognitive_policy.clone(),
            self.cognitive_ledger_dir.clone(),
        ) else {
            return;
        };
        let input = ShadowTurnInput {
            user_text: user_input.to_owned(),
            session_id: self.config.session_id.clone(),
            turn_number: self.turn_number,
            surface: CognitiveSurface::Tui,
            working_dir: self.config.working_dir.clone(),
            world_model_state: self.cognitive_world_model_state.clone(),
            model_id: self.active_model().await,
        };
        let budget_ms = config.max_pipeline_ms;
        let observed = bounded_cognitive_observation(budget_ms, "cognitive-shadow-observe", {
            let ledger_dir = ledger_dir.clone();
            move || {
                let store = archon_cognitive::PersistentCognitiveStore::open(&ledger_dir)?;
                ShadowTurnObserver::new(store.db(), &ledger_dir, config, Some(policy))
                    .observe(input)
            }
        })
        .await;
        match observed {
            Some(Ok(Some(observation))) => tracing::debug!(
                shadow_decision_id = %observation.shadow_decision_id,
                situation = observation.situation_kind.as_str(),
                selected_action = ?observation.selected_action,
                "cognitive shadow observation recorded"
            ),
            Some(Ok(None)) => tracing::debug!("cognitive shadow observer had nothing to plan"),
            Some(Err(error)) => tracing::warn!(%error, "cognitive shadow observation failed"),
            None => {}
        }
    }

    /// Join the finished turn to its shadow plan and reflect if a trigger fired.
    ///
    /// The label comes from what the live turn actually did; the no-op shadow
    /// executor contributes nothing to it.
    pub(super) async fn complete_cognitive_shadow_turn(
        &mut self,
        user_input: &str,
        user_corrected: bool,
    ) {
        let (Some(config), Some(policy), Some(ledger_dir)) = (
            self.cognitive_config.clone(),
            self.cognitive_policy.clone(),
            self.cognitive_ledger_dir.clone(),
        ) else {
            return;
        };
        let Some(situation_kind) = self.current_situation.as_ref().map(|s| s.kind) else {
            return;
        };
        let (tool_names, tool_failures) = turn_tool_activity(&self.state.messages, user_input);
        let session_id = self.config.session_id.clone();
        let turn_number = self.turn_number;
        let live = LiveTurnOutcome {
            live_action_id: format!("{session_id}:{turn_number}"),
            observed_action: observed_action_from_tools(&tool_names),
            completed: true,
            tool_failures,
            user_corrected,
        };
        let correction_confidence = user_corrected.then_some(KEYWORD_CORRECTION_CONFIDENCE);
        let model_id = self.active_model().await;
        let budget_ms = config.max_pipeline_ms;

        let joined = bounded_cognitive_observation(budget_ms, "cognitive-shadow-join", move || {
            let store = archon_cognitive::PersistentCognitiveStore::open(&ledger_dir)?;
            let observer =
                ShadowTurnObserver::new(store.db(), &ledger_dir, config.clone(), Some(policy));
            let comparison = observer.join(&session_id, turn_number, &live, &model_id)?;
            write_triggered_reflection(
                &store,
                &ledger_dir,
                &config,
                &session_id,
                turn_number,
                TurnSignals {
                    situation_kind,
                    shadow_surprise: comparison.as_ref().and_then(|value| value.surprise),
                    tool_failures,
                    correction_confidence,
                    completed: true,
                },
                comparison.as_ref(),
                live.observed_action,
            )?;
            Ok::<_, archon_cognitive::CognitiveError>(comparison)
        })
        .await;
        match joined {
            Some(Ok(Some(comparison))) => tracing::debug!(
                shadow_decision_id = %comparison.shadow_decision_id,
                agreed = ?comparison.agreed,
                surprise = ?comparison.surprise,
                metric_recorded = comparison.metric_recorded,
                "cognitive shadow turn joined to live outcome"
            ),
            Some(Ok(None)) => tracing::debug!("no shadow plan to join for this turn"),
            Some(Err(error)) => tracing::warn!(%error, "cognitive shadow join failed"),
            None => {}
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

/// Confidence assigned to a correction the keyword detector caught.
///
/// The keyword pass fires on explicit corrective phrasing, so a hit is a strong
/// signal — but not a certainty, because the phrase can appear inside quoted
/// text. Above [`archon_cognitive::HIGH_CONFIDENCE_CORRECTION_MIN`] so it
/// triggers, below 1.0 because it is a heuristic. The semantic pass runs later
/// and out of band, so it is deliberately not a reflection trigger.
const KEYWORD_CORRECTION_CONFIDENCE: f32 = 0.9;

/// Run a cognitive observation off the async runtime under a hard budget.
///
/// Returns `None` when the budget was exceeded or the task panicked. Neither
/// is treated as an error worth failing the turn over: the whole point of a
/// shadow observer is that the user cannot tell whether it ran. The task is not
/// cancelled on timeout — it only writes shadow rows — but the caller stops
/// waiting for it.
pub(super) async fn bounded_cognitive_observation<T, F>(
    budget_ms: u64,
    name: &'static str,
    task: F,
) -> Option<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let bounded = tokio::time::timeout(
        Duration::from_millis(budget_ms),
        archon_observability::spawn_blocking_named(name, task),
    )
    .await;
    match bounded {
        Ok(Ok(value)) => Some(value),
        Ok(Err(error)) => {
            tracing::warn!(%error, name, "cognitive observation task failed");
            None
        }
        Err(_) => {
            tracing::warn!(
                name,
                budget_ms,
                "cognitive observation exceeded its budget; turn continues without it"
            );
            None
        }
    }
}

/// Tool names called during this turn, and how many of their results failed.
///
/// The turn starts at the last user message carrying exactly `user_input`,
/// which `begin_process_turn` appended, so no per-turn index has to be carried
/// on the agent. Falls back to the whole conversation only when that message
/// cannot be found, which would otherwise silently report zero activity.
pub(super) fn turn_tool_activity(
    messages: &[serde_json::Value],
    user_input: &str,
) -> (Vec<String>, u32) {
    let start = messages
        .iter()
        .rposition(|message| {
            message["role"] == "user" && message["content"].as_str() == Some(user_input)
        })
        .unwrap_or(0);
    let mut tool_names = Vec::new();
    let mut failures = 0;
    for message in &messages[start..] {
        let Some(blocks) = message["content"].as_array() else {
            continue;
        };
        for block in blocks {
            match block["type"].as_str() {
                Some("tool_use") => {
                    if let Some(name) = block["name"].as_str() {
                        tool_names.push(name.to_string());
                    }
                }
                Some("tool_result") if block["is_error"] == true => failures += 1,
                _ => {}
            }
        }
    }
    (tool_names, failures)
}

/// Write a reflection when the turn tripped a trigger, and nothing otherwise.
///
/// Only ids and enums cross into the writer, so the persisted record cannot
/// contain the turn's text or the model's reasoning.
#[allow(clippy::too_many_arguments)]
fn write_triggered_reflection(
    store: &archon_cognitive::PersistentCognitiveStore,
    ledger_dir: &std::path::Path,
    config: &archon_cognitive::CognitiveConfig,
    session_id: &str,
    turn_number: u64,
    signals: TurnSignals,
    comparison: Option<&ShadowComparison>,
    observed_action: Option<archon_cognitive::CandidateActionKind>,
) -> Result<(), archon_cognitive::CognitiveError> {
    let Some(triggered) = archon_cognitive::reflection_trigger::evaluate(&signals) else {
        return Ok(());
    };
    // Without a shadow plan there is no decision id to anchor the reflection
    // to, and a reflection with no decision is not auditable. Skip rather than
    // mint a synthetic anchor.
    let Some(comparison) = comparison else {
        return Ok(());
    };
    let writer = ReflectionWriter::new(store.db(), ledger_dir, config.record_reflections)?;
    let outcome = writer.reflect_triggered(TriggeredReflectInput {
        decision_id: comparison.decision_id.clone(),
        session_id: session_id.to_owned(),
        turn_number,
        situation_kind: signals.situation_kind,
        goal_action: comparison.shadow_action,
        observed_action,
        trigger: triggered,
        evidence_refs: vec![
            format!("shadow_decision:{}", comparison.shadow_decision_id),
            format!("cognitive_decision:{}", comparison.decision_id),
        ],
    })?;
    for note in outcome.degraded {
        tracing::warn!(note, "triggered reflection degraded");
    }
    Ok(())
}
