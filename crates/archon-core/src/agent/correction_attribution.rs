//! The live call site for R2 causal attribution.
//!
//! `MetricEventKind::AttributionEvaluated` shipped with the R8 measurement
//! schema and, until this module, had zero emitters. This repository has shipped
//! correct code that nothing called five separate times (#76, #129, #161, plus
//! two unemitted metric kinds), so the wiring matters as much as the engine:
//! everything here exists to get one real `attribution_evaluated` row written
//! from a real correction on the real turn path.
//!
//! This file holds the conversation half: reconstructing what the previous
//! turns actually did. `state.messages` is the only record of that which
//! survives the turn -- there is no tool-run log on the agent, and
//! `ToolRunAttemptOutcome` is pushed to a callback nobody keeps -- so the
//! eligible actions come from the `tool_use`/`tool_result` pairs in the
//! transcript, and the tool registry supplies each one's effect class from the
//! tool's own declared permission level.
//!
//! The deciding and recording halves live in [`decide`], split apart there for a
//! reason that cost a CI failure to learn: deciding must be free of side effects
//! so it can be abandoned safely, and the row must be written only after the
//! effect it claims has happened.

use std::path::PathBuf;

use archon_cognitive::attribution::candidates::ATTRIBUTION_LOOKBACK_TURNS;
use archon_cognitive::attribution::input::{
    ActionEffectClass, CorrectionUnderReview, ObservedToolRun,
};
use archon_consciousness::correction_provenance::CorrectionProvenance;

/// Ledger file the executive advisory writes decisions to.
///
/// Mirrors `archon_cognitive::executive_support`, which is where the writer is.
const DECISION_LEDGER_FILE: &str = "cognitive-decisions.jsonl";

/// How many recent decisions to read for one attribution.
///
/// The engine only looks two turns back, so this is a bound on the query rather
/// than a window: a handful covers the lookback several times over even when a
/// turn produced more than one decision.
const RECENT_DECISION_LIMIT: usize = 8;

/// Bound on the tool input rendered into a candidate's lexical text.
const MAX_INPUT_SUMMARY_CHARS: usize = 160;

/// Bound on the decision summary carried into a candidate.
const MAX_DECISION_SUMMARY_CHARS: usize = 200;

/// Prefix every refused tool result starts with.
///
/// Written by `tool_preflight_gates.rs` ("Permission denied for tool '{}'...")
/// and `tool_preflight_steps.rs` ("Permission denied: {reason}"). Both are
/// errors, so this is what tells a refusal apart from a tool that ran and
/// failed -- a distinction the scoring treats as different evidence.
const PERMISSION_DENIED_PREFIX: &str = "Permission denied";

/// Everything gathered on the turn thread, ready to hand to the blocking pool.
pub(super) struct AttributionObservation {
    pub session_id: String,
    pub task_class: String,
    pub model_id: String,
    pub provenance: CorrectionProvenance,
    /// The stored correction text -- already bounded by `stored_correction_content`.
    pub correction_content: String,
    pub tool_runs: Vec<ObservedToolRun>,
    pub ledger_dir: Option<PathBuf>,
}

impl AttributionObservation {
    /// The correction as the engine sees it.
    ///
    /// Session identity comes from the live turn because the correction record
    /// has no session field; the engine then refuses every candidate that does
    /// not match it.
    fn correction_under_review(&self) -> CorrectionUnderReview {
        CorrectionUnderReview {
            correction_id: self.provenance.correction_id.clone(),
            session_id: self.session_id.clone(),
            // `unwrap_or(0)` is not a fallback: zero is the engine's
            // "provenance incomplete" precondition failure, so an unparseable
            // context produces an unattributed row rather than an attribution
            // against whatever happens to be in the window.
            turn_number: self.provenance.turn_number.unwrap_or(0),
            correction_type_code: self.provenance.correction_type.as_code().to_string(),
            summary: self.correction_content.clone(),
            recorded_at: self.provenance.recorded_at,
        }
    }
}

/// Effect class of one observed tool call.
///
/// Read from the tool's own `permission_level`, so the classification tracks the
/// registry rather than a name list that would drift from it. A tool that is no
/// longer registered is `Unknown`, never `Read`: "we could not tell" must not
/// look like "it changed nothing".
pub(super) fn effect_class_of(
    registry: &crate::dispatch::ToolRegistry,
    tool_name: &str,
    input: &serde_json::Value,
) -> ActionEffectClass {
    use archon_tools::tool::PermissionLevel;
    match registry.get(tool_name) {
        Some(tool) => match tool.permission_level(input) {
            PermissionLevel::Safe => ActionEffectClass::Read,
            PermissionLevel::Risky | PermissionLevel::Dangerous => ActionEffectClass::Mutate,
        },
        None => ActionEffectClass::Unknown,
    }
}

/// Index of every tool result in the conversation, by tool-use id.
fn tool_results(messages: &[serde_json::Value]) -> std::collections::HashMap<&str, (bool, &str)> {
    let mut results = std::collections::HashMap::new();
    for message in messages {
        let Some(blocks) = message["content"].as_array() else {
            continue;
        };
        for block in blocks {
            if block["type"].as_str() != Some("tool_result") {
                continue;
            }
            let Some(tool_use_id) = block["tool_use_id"].as_str() else {
                continue;
            };
            let is_error = block["is_error"] == true;
            let content = block["content"].as_str().unwrap_or("");
            results.insert(tool_use_id, (is_error, content));
        }
    }
    results
}

fn bounded(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// Reconstruct the tool runs of the turns preceding `correction_turn`.
///
/// Turn boundaries are the user messages whose `content` is a plain string --
/// the shape `ConversationState::add_user_message` writes, and the one thing
/// that reliably separates turns, since tool results are appended as user
/// messages with array content. Turn numbers are assigned by counting back from
/// the correction's own turn, which assumes the boundaries are contiguous; a
/// compaction that rewrote the transcript would shift them, and the session
/// filter plus the two-turn lookback are what keep that from reaching across
/// into another session's actions.
pub(super) fn observed_tool_runs(
    messages: &[serde_json::Value],
    session_id: &str,
    correction_turn: u64,
    effect_class: &dyn Fn(&str, &serde_json::Value) -> ActionEffectClass,
) -> Vec<ObservedToolRun> {
    let boundaries: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message["role"] == "user" && message["content"].is_string())
        .map(|(index, _)| index)
        .collect();
    if boundaries.len() < 2 {
        // Fewer than two user turns means nothing has completed yet, so there is
        // no prior turn to attribute anything to.
        return Vec::new();
    }

    let results = tool_results(messages);
    let mut runs = Vec::new();
    for back in 1..=ATTRIBUTION_LOOKBACK_TURNS as usize {
        if boundaries.len() < back + 1 {
            break;
        }
        let Some(turn_number) = correction_turn.checked_sub(back as u64) else {
            break;
        };
        if turn_number == 0 {
            break;
        }
        let start = boundaries[boundaries.len() - 1 - back];
        let end = boundaries[boundaries.len() - back];
        let mut ordinal = 0u32;
        for message in &messages[start..end] {
            let Some(blocks) = message["content"].as_array() else {
                continue;
            };
            for block in blocks {
                if block["type"].as_str() != Some("tool_use") {
                    continue;
                }
                let (Some(tool_use_id), Some(tool_name)) =
                    (block["id"].as_str(), block["name"].as_str())
                else {
                    continue;
                };
                let input = &block["input"];
                let (failed, content) = results.get(tool_use_id).copied().unwrap_or((false, ""));
                runs.push(ObservedToolRun {
                    session_id: session_id.to_string(),
                    turn_number,
                    ordinal,
                    tool_use_id: tool_use_id.to_string(),
                    // The transcript records one result per tool-use id; a
                    // retry is dispatched under a new id, so within this view
                    // every run is its first attempt.
                    attempt: 1,
                    tool_name: tool_name.to_string(),
                    input_summary: bounded(&input.to_string(), MAX_INPUT_SUMMARY_CHARS),
                    effect_class: effect_class(tool_name, input),
                    failed,
                    blocked: failed && content.starts_with(PERMISSION_DENIED_PREFIX),
                });
                ordinal += 1;
            }
        }
    }
    runs
}

#[path = "correction_attribution/decide.rs"]
mod decide;

pub(super) use decide::{
    AttributionPlan, AttributionVerdict, commit_correction_attribution, plan_correction_attribution,
};

impl super::Agent {
    /// Decide a just-recorded correction's attribution, without writing
    /// anything.
    ///
    /// Runs BEFORE any rule score moves. That is the inversion R2 item 5 asks
    /// for: reinforcement is a claim that this correction was caused by
    /// something, and until attribution says what, there is nothing to
    /// reinforce on the strength of.
    ///
    /// Deliberately NOT under a wall-clock budget, and not on
    /// `bounded_cognitive_observation`. That helper abandons its task on timeout
    /// without cancelling it, which is safe for a closure that only writes rows
    /// nothing acts on. It was not safe here: the earlier version decided AND
    /// wrote inside the abandoned task, so an expired budget produced a row
    /// saying `accepted=true` next to a reinforcement that never happened --
    /// precisely the "the row claims an effect that did not occur" failure the
    /// pending-adjudication sentinel exists to prevent, arriving by another
    /// door. Splitting the decision out removes the race rather than shortening
    /// it: this half has no side effects to abandon.
    ///
    /// The turn is already exposed to unbounded graph latency three lines
    /// earlier, where `record_correction_unreinforced` writes synchronously, so
    /// a budget on this bounded read while that write is unbounded bought
    /// nothing but a way to lose a reinforcement to a busy machine.
    ///
    /// `None` means there is nothing to decide: no store, not a high-confidence
    /// correction, or not the immediate detection pass.
    pub(super) async fn plan_correction_attribution(
        &self,
        correction: &archon_consciousness::corrections::Correction,
        classification: &archon_consciousness::correction_classifier::CorrectionClassification,
    ) -> Option<AttributionPlan> {
        let Some(store) = self.cognitive_store.as_ref().map(std::sync::Arc::clone) else {
            // Worth a warning rather than a debug line: without the cognitive
            // store there is no attribution, and therefore no correction-driven
            // rule reinforcement at all in this process.
            tracing::warn!(
                "no cognitive store; R2 attribution unavailable, so no correction reinforces a rule"
            );
            return None;
        };
        // Slice 3 step 1: attribution runs on a HIGH-CONFIDENCE correction. An
        // abstention or a low-confidence label is not a correction the R3
        // interface is willing to stand behind, and attributing one would build
        // the R2 corpus on the R3 corpus's rejects.
        if classification.abstained()
            || !classification.is_correction
            || classification.confidence < archon_cognitive::HIGH_CONFIDENCE_CORRECTION_MIN
        {
            return None;
        }

        let provenance = CorrectionProvenance::from_record(correction);
        if provenance.pass != archon_consciousness::correction_provenance::CorrectionPass::Immediate
        {
            // The deferred semantic pass records against a turn whose actions
            // have already left the window this reconstructs. Refusing beats
            // attributing it to whatever is in the transcript now.
            return None;
        }

        let session_id = self.config.session_id.clone();
        let tool_runs = observed_tool_runs(
            &self.state.messages,
            &session_id,
            provenance.turn_number.unwrap_or(0),
            &|tool_name, input| effect_class_of(&self.registry, tool_name, input),
        );
        let observation = AttributionObservation {
            session_id,
            task_class: self
                .current_situation
                .as_ref()
                .map_or("unclassified", |situation| situation.kind.as_str())
                .to_string(),
            model_id: self.config.model.clone(),
            provenance,
            correction_content: correction.content.clone(),
            tool_runs,
            ledger_dir: self.cognitive_ledger_dir.clone(),
        };

        // Off the async runtime, and awaited to completion rather than raced
        // against a clock. A join failure yields no plan, which withholds the
        // reinforcement -- the fail-closed direction.
        match archon_observability::spawn_blocking_named("plan-correction-attribution", move || {
            let store = store
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            plan_correction_attribution(&store, &observation)
        })
        .await
        {
            Ok(plan) => Some(plan),
            Err(error) => {
                tracing::warn!(%error, "R2 attribution planning failed; withholding reinforcement");
                None
            }
        }
    }

    /// Record a decided attribution.
    ///
    /// Called only once the effect the row will claim has already been applied.
    /// Nothing reads the return value except logging: the reinforcement decision
    /// was made from the plan, so a lost commit costs an evaluation and not a
    /// divergence.
    pub(super) async fn commit_correction_attribution(
        &self,
        plan: AttributionPlan,
    ) -> Option<AttributionVerdict> {
        let store = self.cognitive_store.as_ref().map(std::sync::Arc::clone)?;
        match archon_observability::spawn_blocking_named(
            "commit-correction-attribution",
            move || {
                let store = store
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                commit_correction_attribution(&store, &plan)
            },
        )
        .await
        {
            Ok(Ok(verdict)) => {
                tracing::debug!(?verdict, "recorded R2 attribution evaluation");
                Some(verdict)
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "R2 attribution row lost; the evaluation is not in the corpus");
                None
            }
            Err(error) => {
                tracing::warn!(%error, "R2 attribution commit task failed");
                None
            }
        }
    }
}

#[cfg(test)]
#[path = "correction_attribution_tests.rs"]
mod tests;
