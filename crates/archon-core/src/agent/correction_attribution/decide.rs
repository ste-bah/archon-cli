//! Deciding an attribution, and recording one. Two functions, on purpose.
//!
//! They used to be one, inside a task a wall-clock budget could abandon without
//! cancelling it. On a loaded runner the budget expired, the caller withheld the
//! rule reinforcement -- correctly, failing closed -- and the orphaned task
//! carried on and wrote a row saying `accepted=true`. The corpus then held an
//! evaluation of an effect that never happened, and
//! `causal_attribution_precision` is computed over exactly those rows.
//!
//! So the seam runs between the two things that must not share a fate:
//!
//! * [`plan_correction_attribution`] reads the decision ledger and runs the
//!   engine. It writes nothing, which makes abandoning it harmless.
//! * [`commit_correction_attribution`] writes the lesson and the row. The caller
//!   invokes it only after applying the effect the row will claim.
//!
//! The reinforcement decision is read from the plan the caller already owns, not
//! from whether the commit returned, so a lost commit costs an evaluation rather
//! than creating a disagreement between the row and the world.

use std::path::Path;

use archon_cognitive::attribution::event::{attribution_event, attribution_window};
use archon_cognitive::attribution::input::{
    AttributionInput, ObservedDecision, action_kind_from_decision_summary,
};
use archon_cognitive::attribution::{AttributionEngine, CAUSAL_ATTRIBUTION_VERSION};

use super::{
    AttributionObservation, DECISION_LEDGER_FILE, MAX_DECISION_SUMMARY_CHARS,
    RECENT_DECISION_LIMIT, bounded,
};

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
pub(in crate::agent) struct AttributionVerdict {
    /// The one field that authorises a rule reinforcement.
    pub accepted: bool,
    pub cohort: &'static str,
    pub rationale_code: String,
    /// Id of the lesson this attribution derived or corroborated, when it
    /// accepted a cause.
    pub lesson_id: Option<String>,
}

/// A decided attribution that has not been written down yet.
///
/// The separation this type exists for: deciding is read-only, recording is
/// not, and the two must not share a fate with anything that can abandon one
/// half. Everything here is owned plain data, so a plan can cross a task
/// boundary and be acted on by the caller that asked for it.
#[derive(Debug, Clone)]
pub(in crate::agent) struct AttributionPlan {
    input: AttributionInput,
    assessment: archon_cognitive::AttributionAssessment,
    cohort: archon_cognitive::MetricCohort,
    window: archon_cognitive::EvaluationWindow,
    task_class: String,
    model_id: String,
}

impl AttributionPlan {
    /// Whether this plan authorises a rule reinforcement.
    ///
    /// Read from the assessment the caller already owns, NOT from the outcome
    /// of writing the row. That is what keeps the reinforcement and the row that
    /// claims it from being decided by two different things.
    pub(in crate::agent) fn accepted(&self) -> bool {
        self.assessment.attributed
    }

    pub(in crate::agent) fn cohort_code(&self) -> &'static str {
        self.assessment.cohort.as_code()
    }

    pub(in crate::agent) fn rationale_code(&self) -> &str {
        &self.assessment.rationale_code
    }
}

/// Decide one correction's attribution. Reads only.
///
/// Nothing here writes a row, a lesson, or a score, so abandoning this half at
/// any point leaves the world exactly as it found it. The engine is already a
/// pure function; this makes the whole decision phase one.
pub(in crate::agent) fn plan_correction_attribution(
    store: &archon_cognitive::PersistentCognitiveStore,
    observation: &AttributionObservation,
) -> AttributionPlan {
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

    let assessment = AttributionEngine.attribute(&input);
    let window = attribution_window(input.correction.recorded_at);
    let cohort = archon_cognitive::MetricCohort::new(
        observation.task_class.clone(),
        observation.model_id.clone(),
        // Procedure version as the policy axis: a scoring change must not pool
        // with rows measured under the previous one.
        CAUSAL_ATTRIBUTION_VERSION,
    );

    AttributionPlan {
        input,
        assessment,
        cohort,
        window,
        task_class: observation.task_class.clone(),
        model_id: observation.model_id.clone(),
    }
}

/// Write the lesson and the `attribution_evaluated` row for a decided plan.
///
/// Called only after the effect the row will claim has already been applied, so
/// a row asserting `accepted` cannot outlive a reinforcement that did not
/// happen. An `Err` here means the evaluation was lost, which is a different and
/// lesser problem than an evaluation that was recorded but took no effect.
pub(in crate::agent) fn commit_correction_attribution(
    store: &archon_cognitive::PersistentCognitiveStore,
    plan: &AttributionPlan,
) -> Result<AttributionVerdict, archon_cognitive::CognitiveError> {
    // `Lesson -> DerivedFrom -> Correction + evidence`, written before the
    // metric row so the row can name the lesson it produced. A lesson whose
    // provenance matches one already stored is corroborated rather than
    // duplicated.
    let lesson = record_causal_lesson(
        store,
        &plan.input,
        &plan.assessment,
        &plan.task_class,
        &plan.model_id,
    );

    let event_store = archon_cognitive::metrics::MetricEventStore::new(store.db(), store.root())?;
    event_store.declare_window(&plan.window)?;
    event_store.record(&attribution_event(
        &plan.input,
        &plan.assessment,
        plan.cohort.clone(),
        &plan.window,
        lesson.as_deref(),
    ))?;

    Ok(AttributionVerdict {
        accepted: plan.assessment.attributed,
        cohort: plan.assessment.cohort.as_code(),
        rationale_code: plan.assessment.rationale_code.clone(),
        lesson_id: lesson,
    })
}

/// Store the causal lesson for an accepted attribution, deduplicated.
///
/// Returns the lesson id, or `None` when the attribution named no cause -- a
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
