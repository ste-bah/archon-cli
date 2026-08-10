//! The live call site for R2 causal attribution.
//!
//! `MetricEventKind::AttributionEvaluated` shipped with the R8 measurement
//! schema and, until this module, had zero emitters. This repository has shipped
//! correct code that nothing called five separate times (#76, #129, #161, plus
//! two unemitted metric kinds), so the wiring matters as much as the engine:
//! everything here exists to get one real `attribution_evaluated` row written
//! from a real correction on the real turn path.
//!
//! Two halves, split by where the data lives.
//!
//! The conversation half is synchronous and cheap. `state.messages` is the only
//! record of what the agent actually did that survives the turn -- there is no
//! tool-run log on the agent, and `ToolRunAttemptOutcome` is pushed to a callback
//! nobody keeps -- so the eligible actions are reconstructed from the
//! `tool_use`/`tool_result` pairs in the transcript, and the tool registry
//! supplies each one's effect class from the tool's own declared permission
//! level.
//!
//! The store half runs on the blocking pool: it reads the decision ledger,
//! scores, and appends one metric row. It is telemetry, so it fails open. It
//! also runs strictly AFTER the correction has been written and its rule
//! boosted, which is the ordering that makes "attribution influenced nothing"
//! true by construction rather than by inspection.

use std::path::{Path, PathBuf};

use archon_cognitive::attribution::candidates::ATTRIBUTION_LOOKBACK_TURNS;
use archon_cognitive::attribution::event::{attribution_event, attribution_window};
use archon_cognitive::attribution::input::{
    ActionEffectClass, AttributionInput, CorrectionUnderReview, ObservedDecision, ObservedToolRun,
    action_kind_from_decision_summary,
};
use archon_cognitive::attribution::{AttributionEngine, CAUSAL_ATTRIBUTION_VERSION};
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
/// Written by `tool_preflight_gates.rs` ("Permission denied for tool '{}'…")
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

/// Recent decisions for this session, as candidates.
fn observed_decisions(
    store: &archon_cognitive::PersistentCognitiveStore,
    ledger_dir: &Path,
    session_id: &str,
) -> Vec<ObservedDecision> {
    let ledger_path = ledger_dir.join(DECISION_LEDGER_FILE);
    let decisions = archon_cognitive::DecisionStore::new(store.db(), ledger_path)
        .and_then(|decisions| decisions.list_for_session(session_id, RECENT_DECISION_LIMIT));
    match decisions {
        Ok(decisions) => decisions
            .into_iter()
            .map(|decision| ObservedDecision {
                decision_id: decision.decision_id,
                session_id: decision.session_id,
                turn_number: decision.turn_number,
                selected_candidate_id: decision.selected_candidate_id,
                action_kind: action_kind_from_decision_summary(&decision.user_visible_summary)
                    .to_string(),
                summary: bounded(&decision.user_visible_summary, MAX_DECISION_SUMMARY_CHARS),
            })
            .collect(),
        Err(error) => {
            // Degrading to tool-run candidates only is a real loss of evidence,
            // so it is said out loud rather than swallowed: an attribution
            // decided without the decision ledger is a different measurement
            // from one decided with it.
            tracing::warn!(%error, "decision ledger unavailable; attributing over tool runs only");
            Vec::new()
        }
    }
}

/// What the attribution decided, in the form the caller acts on.
///
/// Deliberately small and owned: the caller's only question is whether a
/// reinforcement is warranted, and handing it the whole ranked assessment would
/// invite it to read a cause out of a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AttributionVerdict {
    /// The one field that authorises a rule reinforcement.
    pub accepted: bool,
    pub cohort: &'static str,
    pub rationale_code: String,
    /// Id of the lesson this attribution derived or corroborated, when it
    /// accepted a cause.
    pub lesson_id: Option<String>,
}

/// Score one correction, record its lesson, and append its
/// `attribution_evaluated` row.
///
/// Returns the verdict. An `Err` means the measurement could not be written,
/// and the caller must treat that as "not accepted": a reinforcement applied
/// without a recorded justification is exactly the write this slice removes.
pub(super) fn record_correction_attribution(
    store: &archon_cognitive::PersistentCognitiveStore,
    observation: &AttributionObservation,
) -> Result<AttributionVerdict, archon_cognitive::CognitiveError> {
    let correction = observation.correction_under_review();
    let decisions = observation
        .ledger_dir
        .as_deref()
        .map(|ledger_dir| observed_decisions(store, ledger_dir, &observation.session_id))
        .unwrap_or_default();
    let input = AttributionInput {
        correction,
        tool_runs: observation.tool_runs.clone(),
        decisions,
    };

    // The engine sees owned data and returns a verdict. Nothing mutable is in
    // scope here except the cognitive store, which appends measurement rows and
    // causal lessons; no rule and no memory graph is reachable from here.
    let assessment = AttributionEngine.attribute(&input);
    let window = attribution_window(input.correction.recorded_at);
    let cohort = archon_cognitive::MetricCohort::new(
        observation.task_class.clone(),
        observation.model_id.clone(),
        // Procedure version as the policy axis: a scoring change must not pool
        // with rows measured under the previous one.
        CAUSAL_ATTRIBUTION_VERSION,
    );

    // `Lesson -> DerivedFrom -> Correction + evidence`, written before the
    // metric row so the row can name the lesson it produced. A lesson whose
    // provenance matches one already stored is corroborated rather than
    // duplicated.
    let lesson = record_causal_lesson(
        store,
        &input,
        &assessment,
        &observation.task_class,
        &observation.model_id,
    );

    let event_store = archon_cognitive::metrics::MetricEventStore::new(store.db(), store.root())?;
    event_store.declare_window(&window)?;
    event_store.record(&attribution_event(
        &input,
        &assessment,
        cohort,
        &window,
        lesson.as_deref(),
    ))?;

    Ok(AttributionVerdict {
        accepted: assessment.attributed,
        cohort: assessment.cohort.as_code(),
        rationale_code: assessment.rationale_code.clone(),
        lesson_id: lesson,
    })
}

/// Store the causal lesson for an accepted attribution, deduplicated.
///
/// Returns the lesson id, or `None` when the attribution named no cause — a
/// refusal has nothing to derive a lesson from, and minting one anyway would
/// put an unexplained correction into the lesson corpus.
fn record_causal_lesson(
    store: &archon_cognitive::PersistentCognitiveStore,
    input: &AttributionInput,
    assessment: &archon_cognitive::AttributionAssessment,
    task_class: &str,
    model_id: &str,
) -> Option<String> {
    let lesson = archon_cognitive::attribution::lesson::causal_lesson(
        input, assessment, task_class, model_id,
    )?;
    match archon_cognitive::attribution::lesson::record_causal_lesson(store.db(), &lesson) {
        Ok(outcome) => {
            tracing::debug!(?outcome, lesson_id = %outcome.lesson_id(), "recorded causal lesson");
            Some(outcome.into_lesson_id())
        }
        Err(error) => {
            // The metric row is still written without a lesson id. Reporting a
            // lesson that is not in the store would make the join integrity the
            // R2 gate requires unverifiable.
            tracing::warn!(%error, "causal lesson write failed; attribution row carries no lesson");
            None
        }
    }
}

/// Wall-clock budget for one attribution.
///
/// It reads the decision ledger and appends two rows, so it is not free, and it
/// now sits between a user's correction and the end of their turn. Exceeding
/// the budget yields no verdict, which means no reinforcement: the roadmap
/// forbids state mutation from failing open, so a slow store withholds a rule
/// boost rather than waving one through unmeasured.
const ATTRIBUTION_BUDGET_MS: u64 = 750;

impl super::Agent {
    /// Attribute a just-recorded correction and report whether it earned a
    /// reinforcement.
    ///
    /// Runs BEFORE any rule score moves. That is the inversion R2 item 5 asks
    /// for: reinforcement is a claim that this correction was caused by
    /// something, and until attribution says what, there is nothing to
    /// reinforce on the strength of. The verdict is the only thing the caller
    /// may act on, and `accepted` is the only field that authorises a write.
    ///
    /// `None` means no verdict — no store, not a high-confidence correction,
    /// the budget expired, or the write failed. Every one of those is a refusal
    /// to reinforce, never a licence to.
    pub(super) async fn attribute_correction(
        &self,
        correction: &archon_consciousness::corrections::Correction,
        classification: &archon_consciousness::correction_classifier::CorrectionClassification,
    ) -> Option<AttributionVerdict> {
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

        // Off the async runtime and under a hard budget, like every other
        // cognitive observation on this path. Unlike them, the caller waits for
        // the answer, because the answer gates a write.
        super::cognitive_gate::bounded_cognitive_observation(
            ATTRIBUTION_BUDGET_MS,
            "record-correction-attribution",
            move || {
                let store = store
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                match record_correction_attribution(&store, &observation) {
                    Ok(verdict) => {
                        tracing::debug!(?verdict, "recorded R2 attribution evaluation");
                        Some(verdict)
                    }
                    Err(error) => {
                        tracing::warn!(%error, "R2 attribution write failed; withholding reinforcement");
                        None
                    }
                }
            },
        )
        .await
        .flatten()
    }
}

#[cfg(test)]
#[path = "correction_attribution_tests.rs"]
mod tests;
