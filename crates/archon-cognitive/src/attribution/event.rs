//! The `attribution_evaluated` row.
//!
//! `MetricEventKind::AttributionEvaluated` declares nine mandatory identity
//! keys and the metric store rejects an event missing any of them at write
//! time. Three of those keys name things that do not exist yet for a shadow
//! run -- the adjudication, the adjudicated candidate, and, for an unattributed
//! outcome, the decision and action attempt. They are filled with prefixed
//! sentinels rather than plausible-looking ids:
//!
//! * a sentinel joins to nothing, so a downstream query that assumed a real id
//!   returns empty instead of silently matching the wrong row;
//! * the prefix is greppable, so "how many rows are still unadjudicated" is a
//!   string match rather than a schema change;
//! * `pending_adjudication:*` can never equal a proposed `causal_candidate_id`,
//!   and the roadmap defines an accepted link as correct only when those two are
//!   equal. So precision computed over today's rows reads as zero correct out of
//!   N proposed. That is the honest number for a corpus nobody has adjudicated,
//!   and it is why this module does not default the adjudicated id to the
//!   proposed one.

use chrono::{DateTime, Duration, Utc};

use crate::attribution::input::AttributionInput;
use crate::attribution::{ATTRIBUTION_MODE, AttributionAssessment, CAUSAL_ATTRIBUTION_VERSION};
use crate::metrics::event::{CognitiveMetricEvent, MetricEventKind};
use crate::metrics::window::{EvaluationWindow, MetricCohort};

/// Metric name carried by every shadow attribution row.
pub const ATTRIBUTION_METRIC: &str = "causal_attribution_shadow_evaluation";

/// Marks these rows as engine output rather than adjudicated ground truth.
pub const ATTRIBUTION_LABEL_SOURCE: &str = "shadow_attribution_engine";

/// Prefix of every identity standing in for an adjudication that has not
/// happened.
pub const ADJUDICATION_PENDING_PREFIX: &str = "pending_adjudication:";

/// Prefix of every identity standing in for a thing that was not observed.
pub const UNOBSERVED_PREFIX: &str = "unobserved:";

/// Prefix of the `causal_candidate_id` written when no cause is claimed.
pub const NO_CAUSE_PREFIX: &str = "no_cause:";

/// Most ranked candidate ids recorded on one row.
const MAX_RECORDED_RANKS: usize = 3;

/// The UTC-day window an attribution belongs to.
///
/// A pure function of the correction's own timestamp, not of "now": windows are
/// immutable once declared, so a definition that moved with the clock would be
/// redeclared with different bounds on the next write and rejected.
pub fn attribution_window(recorded_at: DateTime<Utc>) -> EvaluationWindow {
    let day = recorded_at.date_naive();
    let started_at = day.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc();
    EvaluationWindow::new(
        format!("causal-attribution-{day}"),
        started_at,
        started_at + Duration::days(1),
    )
}

/// Deterministic row identity: one attribution per correction per procedure
/// version.
///
/// The version is in the id because the store rejects a second write under an
/// existing id with different content. Re-running a corpus under a new
/// procedure must produce new rows, not a hard error against the old ones.
pub fn attribution_event_id(correction_id: &str) -> String {
    format!("causal-attribution:{CAUSAL_ATTRIBUTION_VERSION}:{correction_id}")
}

/// Build the `attribution_evaluated` event for one assessment.
pub fn attribution_event(
    input: &AttributionInput,
    assessment: &AttributionAssessment,
    cohort: MetricCohort,
    window: &EvaluationWindow,
) -> CognitiveMetricEvent {
    let correction_id = input.correction.correction_id.as_str();
    let accepted = assessment.accepted_candidate();

    let causal_candidate_id = accepted.map_or_else(
        || format!("{NO_CAUSE_PREFIX}{correction_id}"),
        |scored| scored.candidate.candidate_id.clone(),
    );
    let decision_id = accepted
        .and_then(|scored| scored.candidate.decision_id.clone())
        // An accepted tool-run link still needs a decision id, and the
        // conversation may carry no finalized decision for that turn. Naming the
        // absence beats reusing an unrelated decision.
        .or_else(|| {
            input
                .decisions
                .iter()
                .find(|decision| decision.turn_number < input.correction.turn_number)
                .map(|decision| decision.decision_id.clone())
        })
        .unwrap_or_else(|| format!("{UNOBSERVED_PREFIX}decision:{correction_id}"));
    let action_attempt_id = accepted
        .and_then(|scored| scored.candidate.action_attempt_id.clone())
        .unwrap_or_else(|| format!("{UNOBSERVED_PREFIX}action_attempt:{correction_id}"));

    let mut evidence_refs = vec![
        format!("correction:{correction_id}"),
        format!("session:{}", input.correction.session_id),
        format!("turn:{}", input.correction.turn_number),
        format!("attribution_version:{CAUSAL_ATTRIBUTION_VERSION}"),
    ];
    if let Some(scored) = accepted {
        evidence_refs.extend(scored.candidate.evidence_refs());
        evidence_refs.extend(
            scored
                .evidence_codes()
                .into_iter()
                .map(|code| format!("evidence:{code}")),
        );
    }

    let ranked_ids = assessment
        .ranked
        .iter()
        .take(MAX_RECORDED_RANKS)
        .map(|scored| scored.candidate.candidate_id.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let top_evidence = assessment
        .ranked
        .first()
        .map(|scored| scored.evidence_codes().join(","))
        .unwrap_or_default();

    let mut event = CognitiveMetricEvent::new(
        attribution_event_id(correction_id),
        ATTRIBUTION_METRIC,
        MetricEventKind::AttributionEvaluated,
        window.evaluation_window_id.clone(),
        cohort,
        input.correction.recorded_at,
    )
    .with_session(
        input.correction.session_id.clone(),
        input.correction.turn_number,
    )
    .with_value(f64::from(assessment.confidence))
    // Not a verified outcome: this row says what the engine proposed, not
    // whether it was right. Adjudication and the follow-up window supply that.
    .with_outcome("shadow")
    .with_identity("correction_id", correction_id)
    .with_identity("decision_id", decision_id)
    .with_identity("action_attempt_id", action_attempt_id)
    .with_identity("causal_candidate_id", causal_candidate_id)
    .with_identity(
        "adjudicated_causal_candidate_id",
        format!("{ADJUDICATION_PENDING_PREFIX}{correction_id}"),
    )
    .with_identity(
        "attribution_adjudication_id",
        format!("{ADJUDICATION_PENDING_PREFIX}{correction_id}"),
    )
    .with_identity("cause_action_class", assessment.cause_action_class_code())
    .with_identity("accepted", bool_identity(assessment.attributed))
    .with_identity("abstained", bool_identity(assessment.abstained()))
    .with_identity("attribution_cohort", assessment.cohort.as_code())
    .with_identity("rationale_code", assessment.rationale_code.clone())
    .with_identity("attribution_version", CAUSAL_ATTRIBUTION_VERSION)
    // The R0 gate's shadow-containment claim, made checkable from the row.
    .with_identity("attribution_mode", ATTRIBUTION_MODE.as_code())
    .with_identity("mutation_source", "none")
    .with_identity(
        "correction_type",
        input.correction.correction_type_code.clone(),
    )
    .with_identity("candidate_population", assessment.ranked.len().to_string())
    .with_identity(
        "candidate_rank",
        accepted.map_or_else(|| "none".to_string(), |scored| scored.rank.to_string()),
    )
    .with_identity(
        "tool_use_id",
        accepted
            .and_then(|scored| scored.candidate.tool_use_id.clone())
            .unwrap_or_else(|| format!("{UNOBSERVED_PREFIX}tool_use:{correction_id}")),
    )
    .with_identity(
        "ranked_candidate_ids",
        if ranked_ids.is_empty() {
            "none".to_string()
        } else {
            ranked_ids
        },
    )
    .with_identity(
        "top_candidate_evidence",
        if top_evidence.is_empty() {
            "none".to_string()
        } else {
            top_evidence
        },
    );
    event.evidence_refs = evidence_refs;
    // Set directly: the builder has no setter, and this is what marks the rows
    // as engine output so an adjudication pass can outrank them.
    event.label_source = ATTRIBUTION_LABEL_SOURCE.to_string();
    event
}

fn bool_identity(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}
