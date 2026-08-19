//! Live wiring for the self-model prediction and the reflection feedback loop.
//!
//! A child module of `cognitive_gate` (file-size guard), and deliberately the
//! only place the two halves of issues #80 and #81 meet, because they share one
//! deterministic verdict on the finished turn:
//!
//! * **#80(a)** — before the LLM request, [`Agent::prepare_cognitive_turn_learning`]
//!   records what the self-model expects of this turn. After finalisation, the
//!   same turn's deterministic tool-execution outcome is attached to that
//!   already-written prediction. The ordering is the shadow observer's: predict
//!   first, grade later, never re-derive.
//! * **#80(b)** — on the first turn of a session the injected block is the
//!   self-model briefing: measured domains with their evidence counts, and the
//!   unmeasured ones named as unmeasured.
//! * **#81(a)** — on later turns it is the bounded set of unresolved
//!   reflections relevant to the turn's situation kind.
//! * **#81(b)** — after the turn, each injected reflection is scored: cited
//!   (its marker came back) and verified reuse (cited *and* the deterministic
//!   verification passed). Only the second retires a reflection.
//!
//! Everything here runs inside the existing bounded cognitive observation, so a
//! slow or failing store costs the turn nothing.

use archon_cognitive::reflection_recall::{ReflectionRecall, cited_reflection_ids, render_block};
use archon_cognitive::self_model::briefing;
use archon_cognitive::{
    CognitiveError, PersistentCognitiveStore, SelfModelPredictor, SituationKind, TurnEvidence,
    UnresolvedReflection, domain_for,
};

use super::*;

/// What the pre-action step produced for this turn.
pub(crate) struct TurnLearningContext {
    pub(crate) injected: Vec<UnresolvedReflection>,
    pub(crate) block: Option<String>,
    pub(crate) predicted: Option<f32>,
}

impl Agent {
    /// Record the pre-action self-model prediction and select what to inject.
    ///
    /// Runs before the LLM request. The prediction is written to its own
    /// relation here and never rewritten afterwards, which is what makes the
    /// later comparison a prediction rather than a rationalisation — the same
    /// discipline `run_cognitive_shadow_observation` already establishes.
    pub(crate) async fn prepare_cognitive_turn_learning(&mut self) {
        self.cognitive_learning_block = None;
        self.cognitive_injected_reflections.clear();
        let (Some(config), Some(policy), Some(ledger_dir), Some(situation)) = (
            self.cognitive_config.clone(),
            self.cognitive_policy.clone(),
            self.cognitive_ledger_dir.clone(),
            self.current_situation.clone(),
        ) else {
            return;
        };
        let session_id = self.config.session_id.clone();
        let turn_number = self.turn_number;
        // The briefing is a *startup* briefing: it reports the self-model as it
        // stands when the session begins, and every later turn gets the shorter
        // unresolved-lesson block instead.
        let first_turn = turn_number <= 1;
        let budget_ms = config.max_pipeline_ms;

        let prepared = bounded_cognitive_observation(budget_ms, "cognitive-turn-learning", {
            let ledger_dir = ledger_dir.clone();
            move || {
                let store = PersistentCognitiveStore::open(&ledger_dir)?;
                let predicted =
                    SelfModelPredictor::new(store.db(), &ledger_dir, Some(policy.clone()))?
                        .predict(&session_id, turn_number, domain_for(situation.kind))?
                        .map(|prediction| prediction.predicted_success_probability);

                let recall = ReflectionRecall::new(store.db(), &ledger_dir, Some(policy))?;
                let injected = recall.unresolved_for_turn(&session_id, situation.kind)?;
                // Counted before the turn runs: a turn that dies still consumed
                // one of the reflection's bounded injections.
                recall.record_injection(&session_id, turn_number, &injected)?;

                let block = if first_turn {
                    briefing::build(store.db(), injected.clone())?.render()
                } else {
                    render_block(&injected)
                };
                Ok::<_, CognitiveError>(TurnLearningContext {
                    injected,
                    block,
                    predicted,
                })
            }
        })
        .await;

        match prepared {
            Some(Ok(context)) => {
                tracing::debug!(
                    predicted_success_probability = ?context.predicted,
                    injected_reflections = context.injected.len(),
                    first_turn,
                    "cognitive turn learning prepared"
                );
                self.cognitive_injected_reflections = context.injected;
                self.cognitive_learning_block = context.block;
            }
            Some(Err(error)) => {
                tracing::warn!(%error, "cognitive turn learning preparation failed")
            }
            None => {}
        }
    }

    /// Append the self-model briefing or the unresolved-lesson block.
    pub(crate) fn inject_cognitive_learning(&self, system: &mut Vec<serde_json::Value>) {
        if let Some(ref block) = self.cognitive_learning_block {
            system.push(serde_json::json!({
                "type": "text",
                "text": block,
            }));
        }
    }

    /// Reflections injected into this turn, taken so a later turn cannot
    /// re-score them.
    pub(crate) fn take_injected_reflections(&mut self) -> Vec<UnresolvedReflection> {
        std::mem::take(&mut self.cognitive_injected_reflections)
    }
}

/// Attach the deterministic verification to the turn's prediction and score its
/// injected reflections.
///
/// Called from inside `complete_cognitive_shadow_turn`'s bounded task, which
/// already holds the store, the model id and the turn's tool activity.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_turn_learning(
    store: &PersistentCognitiveStore,
    ledger_dir: &std::path::Path,
    policy: Option<archon_cognitive::CognitivePolicy>,
    session_id: &str,
    turn_number: u64,
    situation_kind: SituationKind,
    evidence: TurnEvidence,
    injected: &[UnresolvedReflection],
    assistant_text: &str,
    model_id: &str,
) -> Result<(), CognitiveError> {
    let resolved = SelfModelPredictor::new(store.db(), ledger_dir, policy.clone())?.resolve(
        session_id,
        turn_number,
        evidence,
        model_id,
    )?;
    if let Some(resolved) = &resolved {
        tracing::debug!(
            prediction_id = %resolved.prediction.prediction_id,
            predicted = resolved.prediction.predicted_success_probability,
            verification = ?resolved.verification,
            metric_recorded = resolved.metric_recorded,
            "self-model prediction verified"
        );
    }

    if injected.is_empty() {
        return Ok(());
    }
    let cited = cited_reflection_ids(assistant_text, injected);
    let tally = ReflectionRecall::new(store.db(), ledger_dir, policy)?.record_outcome(
        &archon_cognitive::reflection_recall::ScoredTurn {
            session_id,
            turn_number,
            model_id,
            situation_kind,
        },
        injected,
        &cited,
        evidence.verdict(),
    )?;
    tracing::debug!(
        injected = tally.injected,
        cited = tally.cited,
        verified_reuse = tally.verified_reuse,
        "injected reflections scored"
    );
    Ok(())
}

#[cfg(test)]
#[path = "cognitive_learning_tests.rs"]
mod tests;

/// The assistant's own output for this turn, which is where a citation marker
/// would appear.
///
/// Bounded to the current turn by the same anchor `turn_tool_activity` uses:
/// the last user message carrying exactly `user_input`. Counting an earlier
/// turn's text would credit a lesson with a citation it never received.
pub(crate) fn turn_assistant_text(messages: &[serde_json::Value], user_input: &str) -> String {
    let start = messages
        .iter()
        .rposition(|message| {
            message["role"] == "user" && message["content"].as_str() == Some(user_input)
        })
        .unwrap_or(0);
    let mut text = String::new();
    for message in &messages[start..] {
        if message["role"] != "assistant" {
            continue;
        }
        let Some(blocks) = message["content"].as_array() else {
            continue;
        };
        for block in blocks {
            if block["type"] == "text"
                && let Some(chunk) = block["text"].as_str()
            {
                text.push_str(chunk);
                text.push('\n');
            }
        }
    }
    text
}
