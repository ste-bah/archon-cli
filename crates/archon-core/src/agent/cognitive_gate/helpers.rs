//! Free helpers for the cognitive gate: the trigger's confidence rule, the
//! observation budget, the turn's tool activity, and the reflection writer.
//!
//! Split from the parent under the file-size guard. These are the parts with
//! no `&self` — the gate's decisions about what may be recorded, separated
//! from the agent state that acts on them.

use super::*;

/// How confident the reflection trigger may be that this turn was a correction.
///
/// This used to be the constant `0.9` for every corrected turn, because the
/// per-correction confidence lived behind a classifier this crate did not yet
/// call. It does now (`memory_integration_corrections.rs`), so the real number
/// is threaded through and the constant is gone.
///
/// `None` unless the classifier positively asserted a correction. Two negatives
/// must stay out, for different reasons:
///
/// * an `abstain.*` rationale is a *declined* answer, not a weak yes. Its
///   confidence describes nothing, so passing it on would let the trigger read
///   "I don't know" as a low-confidence correction;
/// * a confident `is_correction: false` is an answer, and its confidence
///   measures the strength of that *no*. Passing it through would make a
///   classifier certain the user corrected nothing look exactly like one
///   certain they did — the 0.9 constant's failure mode, inverted.
///
/// Gated on the live heuristic having recorded a correction as well. The
/// classifier is shadow-only until its promotion gate passes (learning roadmap
/// line 300), so its job here is to say how strong a correction the live path
/// already accepted was, not to arm the trigger by itself.
pub(crate) fn correction_trigger_confidence(
    user_corrected: bool,
    classification: Option<&CorrectionClassification>,
) -> Option<f32> {
    let classification = classification.filter(|_| user_corrected)?;
    if classification.abstained() || !classification.is_correction {
        return None;
    }
    Some(classification.confidence)
}

/// Run a cognitive observation off the async runtime under a hard budget.
///
/// Returns `None` when the budget was exceeded or the task panicked. Neither
/// is treated as an error worth failing the turn over: the whole point of a
/// shadow observer is that the user cannot tell whether it ran.
///
/// The task is NOT cancelled on timeout. The caller stops waiting; the closure
/// runs on and its writes still land. So `task` must be safe to ABANDON:
/// side-effect-free, or writing only rows nothing acts on. A closure whose
/// result gates a later write does not qualify — `None` would withhold that
/// write while the orphan recorded that it happened, and the row and the world
/// would disagree. R2 attribution shipped that bug and now decides in one half
/// and records in another, neither of them here.
pub(crate) async fn bounded_cognitive_observation<T, F>(
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
pub(crate) fn turn_tool_activity(
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
pub(crate) fn write_triggered_reflection(
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
