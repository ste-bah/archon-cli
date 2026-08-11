use std::time::Duration;

use archon_consciousness::correction_classifier::CorrectionClassification;

use archon_cognitive::{
    ClassifyInput, CognitiveDecision, CognitiveSurface, ExecutiveAdvisoryInput, ExecutiveLoop,
    LiveTurnOutcome, NoopActionExecutor, NoopLessonSink, ReflectionWriter, ShadowComparison,
    ShadowTurnInput, ShadowTurnObserver, SituationClassifier, ToolGateInput, ToolUseGate,
    ToolVerdict, TriggeredReflectInput, TurnSignals, WorldModelScorer, direct_response_for,
    observed_action_from_tools,
};

use super::*;

/// Issues #80 and #81's live wiring, split out under the file-size guard.
#[path = "cognitive_learning.rs"]
pub(super) mod cognitive_learning;

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

    /// Plan this turn's advisory through [`ExecutiveLoop::run_advisory`].
    ///
    /// The loop is the single advisory implementation (issue #76 follow-up).
    /// The store-less free functions that used to serve this call site had
    /// drifted from it — they planned without the candidate store or the
    /// self-model, and always reported `prediction_unavailable` — and the agent
    /// now holds a cognitive store anyway, so they are gone.
    ///
    /// It runs on the agent's already-open store, off the async runtime, under
    /// the configured pipeline budget. The budget used to be a post-hoc error
    /// inside the planner that discarded work already done; as a timeout it
    /// abandons a slow advisory instead, and the turn proceeds without a
    /// reminder. `run_advisory` persists its own decision, so the turn no
    /// longer detaches a second task to do it — which is also why an abandoned
    /// advisory can still leave a decision row behind: the blocking task is not
    /// cancelled, only stopped being waited on.
    pub(super) async fn run_cognitive_executive_advisory(&mut self) {
        self.cognitive_executive_reminder = None;
        let (Some(config), Some(policy), Some(ledger_dir), Some(store), Some(situation)) = (
            self.cognitive_config.clone(),
            self.cognitive_policy.clone(),
            self.cognitive_ledger_dir.clone(),
            self.cognitive_store.clone(),
            self.current_situation.clone(),
        ) else {
            return;
        };
        let input = ExecutiveAdvisoryInput {
            situation,
            working_dir: self.config.working_dir.clone(),
            world_model_state: self.cognitive_world_model_state.clone(),
        };
        // With a backend injected the scorer may consult model predictions;
        // whether it actually does is decided by `world_model_state`, so a
        // shadow-only state still takes the heuristic path. Without a backend
        // there is nothing to consult and the heuristic scorer stands in.
        let backend = self.cognitive_prediction_backend.clone();
        let budget_ms = config.max_pipeline_ms;
        let planned =
            bounded_cognitive_observation(budget_ms, "cognitive-executive-advisory", move || {
                // The handle the agent already holds. Opening a second one per
                // turn is the dominant cost of the whole advisory and would put
                // it at the mercy of the budget on a loaded machine — which
                // costs the prompt its reminder, not just a measurement.
                let store = store
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                match backend {
                    Some(backend) => ExecutiveLoop::with_components(
                        store.db(),
                        config,
                        Some(policy),
                        &ledger_dir,
                        WorldModelScorer::new(backend, true, true),
                        NoopActionExecutor,
                        NoopLessonSink,
                    )?
                    .run_advisory(input),
                    None => ExecutiveLoop::new(store.db(), config, Some(policy), &ledger_dir)?
                        .run_advisory(input),
                }
            })
            .await;
        let outcome = match planned {
            Some(Ok(outcome)) => outcome,
            Some(Err(error)) => {
                tracing::warn!(%error, "cognitive executive advisory planning failed");
                return;
            }
            None => return,
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
            degraded = ?outcome.snapshot.degraded,
            "cognitive executive advisory recorded"
        );
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
        classification: Option<&CorrectionClassification>,
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
        // Issues #80/#81: the deterministic evidence this turn produced, and
        // the reflections it was shown. Both are read here, on the live path,
        // and handed to the same bounded task that joins the shadow plan.
        let evidence = archon_cognitive::TurnEvidence {
            tool_calls: tool_names.len() as u32,
            tool_failures,
            completed: true,
        };
        let injected = self.take_injected_reflections();
        let assistant_text =
            cognitive_learning::turn_assistant_text(&self.state.messages, user_input);
        let learning_policy = Some(policy.clone());
        let session_id = self.config.session_id.clone();
        let turn_number = self.turn_number;
        let live = LiveTurnOutcome {
            live_action_id: format!("{session_id}:{turn_number}"),
            observed_action: observed_action_from_tools(&tool_names),
            completed: true,
            tool_failures,
            user_corrected,
        };
        let correction_confidence = correction_trigger_confidence(user_corrected, classification);
        let model_id = self.active_model().await;
        let budget_ms = config.max_pipeline_ms;

        let joined = bounded_cognitive_observation(budget_ms, "cognitive-shadow-join", move || {
            let store = archon_cognitive::PersistentCognitiveStore::open(&ledger_dir)?;
            let observer =
                ShadowTurnObserver::new(store.db(), &ledger_dir, config.clone(), Some(policy));
            let comparison = observer.join(&session_id, turn_number, &live, &model_id)?;
            // Ahead of the reflection write on purpose: a failed reflection
            // write must not strand this turn's prediction unresolved, because
            // nothing would ever come back to resolve it.
            cognitive_learning::resolve_turn_learning(
                &store,
                &ledger_dir,
                learning_policy,
                &session_id,
                turn_number,
                situation_kind,
                evidence,
                &injected,
                &assistant_text,
                &model_id,
            )?;
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

/// The gate's free helpers, split out under the file-size guard.
#[path = "cognitive_gate/helpers.rs"]
mod helpers;

pub(super) use helpers::{
    bounded_cognitive_observation, correction_trigger_confidence, turn_tool_activity,
    write_triggered_reflection,
};
