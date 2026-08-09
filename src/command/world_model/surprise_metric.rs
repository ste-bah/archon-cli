//! `surprise_observed` events: the producer behind `latent_surprise_mean/_p95`.
//!
//! Those two definitions sat in `archon-cognitive`'s R8 table with nothing
//! writing their event kind, which makes a metric a promise rather than a
//! measurement. The event requires three identities — `prediction_id`,
//! `action_attempt_id`, `verification_id` — and the cognitive turn loop has
//! none of them: its only verification runs against a no-op executor's empty
//! evidence, and a `VerificationContract` carries a fresh per-plan UUID that
//! identifies a contract, not an adjudication.
//!
//! The world model's guarded-action loop has all three, and they meet on a live
//! turn. `record_guardrail_completion_outcome` joins a finished action to the
//! verification outcomes recorded against it and asks the predictor to score
//! its own prediction against what actually happened; the resulting
//! `WorldGuardrailOutcome` carries the JEPA `latent_surprise`, the
//! `prediction_id` it came from, the guarded `action_id` (the same value
//! `materialize.rs` uses as `action_attempt_id`), and the verification's stable
//! `idempotency_key`. That is the event, unfabricated.
//!
//! Nothing is minted to fill a gap: an action missing any of the three, or
//! whose verification never reached a deterministic pass/fail, records nothing
//! at all. An unfed window is honest; an invented identity is not.

use std::path::Path;

use archon_cognitive::metrics::definitions::{VERIFIED_FAILED, VERIFIED_PASSED};
use archon_cognitive::metrics::{
    CognitiveMetricEvent, EvaluationWindow, MetricCohort, MetricEventKind, MetricEventStore,
    MetricWriteOutcome, runtime_cohort,
};
use archon_cognitive::{CognitiveError, PersistentCognitiveStore};
use archon_world_model::{
    GuardrailFinalStatus, RuntimeTaskClass, VerificationOutcome, VerificationStatus,
    WorldGuardrailOutcome,
};
use chrono::{DateTime, Utc};

/// Metric name carried by every surprise event.
///
/// Shared by `latent_surprise_mean` and `latent_surprise_p95`: both derive from
/// the same rows, differing only in how they collapse `value`.
const LATENT_SURPRISE_METRIC: &str = "latent_surprise";

/// Where the number came from.
///
/// Not a human judgement and not the agent's self-report: the world model
/// scored its own prediction against the observed next state.
const LABEL_SOURCE: &str = "world_model_prediction_outcome";

/// Turn identity for one surprise observation.
///
/// Passed in rather than read from the guardrail record because the cognitive
/// store is per workspace, and only the caller knows which workspace this turn
/// belonged to.
pub(crate) struct LatentSurpriseContext<'a> {
    /// Workspace whose `.archon/cognitive` store the event belongs to — the
    /// same one the session opened, so these rows land beside the correction
    /// and shadow events rather than in a store of their own.
    pub working_dir: &'a Path,
    pub session_id: &'a str,
    pub turn_number: u64,
    pub model_id: &'a str,
}

/// Record one `surprise_observed` event, if the action can anchor one.
///
/// `Ok(None)` means the action had nothing to say — no prediction, no
/// deterministic verification, no finite surprise, or no cognitive store in
/// this workspace. None of those is an error, and none may be papered over.
pub(crate) fn record_latent_surprise(
    context: LatentSurpriseContext<'_>,
    outcome: &WorldGuardrailOutcome,
) -> Result<Option<MetricWriteOutcome>, CognitiveError> {
    let Some(anchor) = SurpriseAnchor::from_outcome(outcome) else {
        return Ok(None);
    };
    let root = context.working_dir.join(".archon").join("cognitive");
    // Deliberately not `create_dir_all`: a workspace with no cognitive store
    // has nothing listening, and a measurement write is no reason to create a
    // database the session never asked for.
    if !root.is_dir() {
        return Ok(None);
    }
    let store = PersistentCognitiveStore::open(&root)?;
    let events = MetricEventStore::new(store.db(), store.root())?;
    let window = daily_window(outcome.created_at);
    // Idempotent for an identical definition, so every turn can assert the
    // window it is about to write into rather than depending on start-up order.
    events.declare_window(&window)?;
    events
        .record(&event(&context, outcome, &anchor, &window))
        .map(Some)
}

/// The three identities and the value, once an action has proved it has them.
struct SurpriseAnchor<'a> {
    surprise: f64,
    prediction_id: &'a str,
    action_attempt_id: &'a str,
    verification: &'a VerificationOutcome,
}

impl<'a> SurpriseAnchor<'a> {
    fn from_outcome(outcome: &'a WorldGuardrailOutcome) -> Option<Self> {
        // A non-finite surprise would poison every downstream mean and
        // percentile; the store rejects it anyway, so it is caught here rather
        // than surfaced as a write failure on the turn path.
        let surprise = outcome.latent_surprise.filter(|value| value.is_finite())?;
        let prediction_id = non_empty(outcome.prediction_id.as_deref())?;
        let action_attempt_id = non_empty(Some(outcome.action_id.as_str()))?;
        let verification = anchoring_verification(&outcome.verification_outcomes)?;
        Some(Self {
            surprise: f64::from(surprise),
            prediction_id,
            action_attempt_id,
            verification,
        })
    }
}

/// The verification this surprise is measured against.
///
/// An action can carry several, so the choice has to be a rule rather than
/// arrival order — the event id must be reproducible from the stored outcome.
/// Only a deterministic pass or fail qualifies: `Skipped`, `NotRun` and
/// `Inconclusive` adjudicate nothing, and anchoring to one would claim the
/// surprise was checked when it was not. A failure outranks a pass because a
/// failure is what decided the action's final status; ties break on the stable
/// idempotency key.
fn anchoring_verification(outcomes: &[VerificationOutcome]) -> Option<&VerificationOutcome> {
    outcomes
        .iter()
        .filter(|outcome| {
            matches!(
                outcome.status,
                VerificationStatus::Passed | VerificationStatus::Failed
            ) && !outcome.idempotency_key.trim().is_empty()
        })
        .min_by(|left, right| {
            let decisive =
                |outcome: &VerificationOutcome| outcome.status != VerificationStatus::Failed;
            decisive(left)
                .cmp(&decisive(right))
                .then_with(|| left.idempotency_key.cmp(&right.idempotency_key))
        })
}

fn event(
    context: &LatentSurpriseContext<'_>,
    outcome: &WorldGuardrailOutcome,
    anchor: &SurpriseAnchor<'_>,
    window: &EvaluationWindow,
) -> CognitiveMetricEvent {
    let mut event = CognitiveMetricEvent::new(
        // Derived from the outcome, not random: a retried write is recognised
        // as a replay instead of adding a second row for one observation.
        format!("{LATENT_SURPRISE_METRIC}:{}", outcome.outcome_id),
        LATENT_SURPRISE_METRIC,
        MetricEventKind::SurpriseObserved,
        window.evaluation_window_id.clone(),
        cohort(context, outcome),
        outcome.created_at,
    )
    .with_session(context.session_id, context.turn_number)
    .with_value(anchor.surprise)
    .with_outcome(outcome_status(anchor.verification.status))
    .with_identity("prediction_id", anchor.prediction_id)
    .with_identity("action_attempt_id", anchor.action_attempt_id)
    .with_identity(
        "verification_id",
        anchor.verification.idempotency_key.as_str(),
    )
    .with_identity(
        "verification_kind",
        format!("{:?}", anchor.verification.kind),
    )
    .with_identity(
        "guardrail_final_status",
        final_status_code(outcome.final_status),
    )
    // Archon's own behaviour is part of the data-generating process, so a
    // surprise measured under one build is not comparable with another's.
    .with_identity("world_model_build", archon_world_model::build_stamp());
    event.label_source = LABEL_SOURCE.to_string();
    event.evidence_refs = vec![
        format!("guarded_action:{}", anchor.action_attempt_id),
        format!("world_prediction:{}", anchor.prediction_id),
        format!("verification:{}", anchor.verification.idempotency_key),
    ];
    event
}

fn cohort(context: &LatentSurpriseContext<'_>, outcome: &WorldGuardrailOutcome) -> MetricCohort {
    // The cognitive policy in force, so a surprise measured under one policy is
    // never pooled with another's. Fails open to "no_policy": an unreadable
    // policy is a reason to segment conservatively, not to drop the row.
    let policy = archon_policy::load_effective_policy(context.working_dir)
        .ok()
        .map(|effective| effective.cognitive);
    runtime_cohort(
        task_class_code(outcome.task_class),
        context.model_id,
        policy.as_ref(),
    )
}

/// A UTC day.
///
/// Windows are immutable once declared, so the definition has to be a pure
/// function of the date — a window derived from "now" would be redeclared with
/// different bounds on the next turn and rejected.
fn daily_window(observed_at: DateTime<Utc>) -> EvaluationWindow {
    let day = observed_at.date_naive();
    let started_at = day.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc();
    EvaluationWindow::new(
        format!("latent-surprise-{day}"),
        started_at,
        started_at + chrono::Duration::days(1),
    )
}

/// Only ever called with a deterministic verification, by construction.
fn outcome_status(status: VerificationStatus) -> &'static str {
    match status {
        VerificationStatus::Failed => VERIFIED_FAILED,
        _ => VERIFIED_PASSED,
    }
}

// Stable snake_case codes, spelled out rather than taken from `Debug` or a
// serde rename: these are written into append-only measurement rows, so a
// rename upstream must not silently split a cohort in two.

fn task_class_code(task_class: RuntimeTaskClass) -> &'static str {
    match task_class {
        RuntimeTaskClass::GeneralAnswer => "general_answer",
        RuntimeTaskClass::CodingChange => "coding_change",
        RuntimeTaskClass::ResearchAnswer => "research_answer",
        RuntimeTaskClass::Refactor => "refactor",
        RuntimeTaskClass::Debugging => "debugging",
        RuntimeTaskClass::DataMutation => "data_mutation",
        RuntimeTaskClass::ExternalSideEffect => "external_side_effect",
        RuntimeTaskClass::PipelineExecution => "pipeline_execution",
        RuntimeTaskClass::VerificationOnly => "verification_only",
    }
}

fn final_status_code(status: GuardrailFinalStatus) -> &'static str {
    match status {
        GuardrailFinalStatus::CompletedVerified => "completed_verified",
        GuardrailFinalStatus::CompletedWithCaveat => "completed_with_caveat",
        GuardrailFinalStatus::BlockedMissingVerification => "blocked_missing_verification",
        GuardrailFinalStatus::BlockedFailedVerification => "blocked_failed_verification",
        GuardrailFinalStatus::UserApprovedDespiteRisk => "user_approved_despite_risk",
        GuardrailFinalStatus::UserAborted => "user_aborted",
        GuardrailFinalStatus::Failed => "failed",
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
#[path = "surprise_metric_tests.rs"]
mod tests;
