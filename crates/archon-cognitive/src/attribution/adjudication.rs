//! The human verdict on an attribution.
//!
//! Every shadow attribution row carries `adjudicated_causal_candidate_id =
//! pending_adjudication:*`, which can never equal the candidate the engine
//! proposed. That is deliberate — precision computed over an unadjudicated
//! corpus must read as zero correct, not as a fabricated 1.0 — but it also means
//! precision stays undefined until somebody says what the right answer was.
//! This is where they say it.
//!
//! The adjudication is a NEW row, not an edit. Metric events are append-only and
//! the store rejects a second write under an existing id with different content,
//! so a verdict cannot rewrite the engine's claim; it stands beside it, carrying
//! `label_source = "human_adjudication"` and an `adjudication_scope` that names
//! which arm the original verdict was in. `causal_attribution_precision` is
//! defined over rows whose scope is `accepted`, so the engine's own proposals
//! can never enter their own precision denominator.
//!
//! One adjudication per correction. The id is derived from the correction, so a
//! re-run of the same verdict is a replay and a *different* verdict for a
//! correction already adjudicated is rejected by the store rather than silently
//! overwriting the record of what the first adjudicator said.

use chrono::{DateTime, Utc};
use cozo::DbInstance;

use crate::CognitiveError;
use crate::attribution::CAUSAL_ATTRIBUTION_VERSION;
use crate::attribution::event::{ADJUDICATION_PENDING_PREFIX, NO_CAUSE_PREFIX, attribution_window};
use crate::metrics::event::{CognitiveMetricEvent, MetricEventKind};
use crate::metrics::event_store::{MetricEventStore, MetricWriteOutcome};
use crate::metrics::window::MetricCohort;

/// Metric name carried by every adjudication row.
pub const ADJUDICATION_METRIC: &str = "causal_attribution_adjudication";

/// Marks a row as a human verdict rather than engine output.
pub const ADJUDICATION_LABEL_SOURCE: &str = "human_adjudication";

/// Identity naming which arm of the engine's verdict was adjudicated.
///
/// `causal_attribution_precision` is defined over `accepted` only, because the
/// roadmap's precision is accepted-link precision.
pub const ADJUDICATION_SCOPE: &str = "adjudication_scope";

/// One attribution waiting for a verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAttribution {
    pub correction_id: String,
    pub session_id: String,
    pub turn_number: u64,
    pub proposed_candidate_id: String,
    pub cause_action_class: String,
    pub attribution_cohort: String,
    pub rationale_code: String,
    /// The engine's ranked candidates, best first, as recorded on the row. This
    /// is what an adjudicator picks from.
    pub ranked_candidate_ids: Vec<String>,
    pub decision_id: String,
    pub action_attempt_id: String,
    pub lesson_id: String,
    pub cohort: MetricCohort,
    pub recorded_at: DateTime<Utc>,
}

impl PendingAttribution {
    /// The id every adjudication of this correction carries.
    pub fn adjudication_id(&self) -> String {
        format!("adj:{CAUSAL_ATTRIBUTION_VERSION}:{}", self.correction_id)
    }

    /// The verdict meaning "the engine named a cause and there wasn't one".
    pub fn no_cause_verdict(&self) -> String {
        format!("{NO_CAUSE_PREFIX}{}", self.correction_id)
    }
}

/// Attributions with no verdict yet, newest first.
pub fn list_pending(
    db: &DbInstance,
    ledger_dir: &std::path::Path,
    limit: usize,
) -> Result<Vec<PendingAttribution>, CognitiveError> {
    let events = MetricEventStore::new(db, ledger_dir)?.events()?;
    let adjudicated: std::collections::BTreeSet<&str> = events
        .iter()
        .filter(|event| event.identity(ADJUDICATION_SCOPE).is_some())
        .filter_map(|event| event.identity("correction_id"))
        .collect();

    let mut pending: Vec<PendingAttribution> = events
        .iter()
        .filter(|event| event.event_kind == MetricEventKind::AttributionEvaluated)
        .filter(|event| event.identity(ADJUDICATION_SCOPE).is_none())
        .filter(|event| {
            event
                .identity("adjudicated_causal_candidate_id")
                .is_some_and(|value| value.starts_with(ADJUDICATION_PENDING_PREFIX))
        })
        .filter(|event| {
            event
                .identity("correction_id")
                .is_some_and(|id| !adjudicated.contains(id))
        })
        .filter_map(pending_from_event)
        .collect();
    pending.sort_by(|left, right| {
        right
            .recorded_at
            .cmp(&left.recorded_at)
            .then_with(|| left.correction_id.cmp(&right.correction_id))
    });
    pending.truncate(limit);
    Ok(pending)
}

/// The one pending attribution for `correction_id`, if it is still pending.
pub fn pending_for(
    db: &DbInstance,
    ledger_dir: &std::path::Path,
    correction_id: &str,
) -> Result<Option<PendingAttribution>, CognitiveError> {
    Ok(list_pending(db, ledger_dir, usize::MAX)?
        .into_iter()
        .find(|pending| pending.correction_id == correction_id))
}

fn pending_from_event(event: &CognitiveMetricEvent) -> Option<PendingAttribution> {
    let ranked = event
        .identity("ranked_candidate_ids")
        .unwrap_or("none")
        .split(',')
        .filter(|value| !value.is_empty() && *value != "none")
        .map(str::to_string)
        .collect();
    Some(PendingAttribution {
        correction_id: event.identity("correction_id")?.to_string(),
        session_id: event.session_id.clone(),
        turn_number: event.turn_number,
        proposed_candidate_id: event.identity("causal_candidate_id")?.to_string(),
        cause_action_class: event.identity("cause_action_class")?.to_string(),
        attribution_cohort: event.identity("attribution_cohort")?.to_string(),
        rationale_code: event.identity("rationale_code").unwrap_or("").to_string(),
        ranked_candidate_ids: ranked,
        decision_id: event.identity("decision_id")?.to_string(),
        action_attempt_id: event.identity("action_attempt_id")?.to_string(),
        lesson_id: event.identity("lesson_id").unwrap_or("").to_string(),
        cohort: event.cohort.clone(),
        recorded_at: event.created_at,
    })
}

/// A human's answer to "what actually caused this correction?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributionVerdict {
    /// The candidate the adjudicator says was the cause. `None` means "nothing
    /// here caused it", which is a real verdict and the one that makes an
    /// accepted link wrong.
    pub adjudicated_candidate_id: Option<String>,
    /// Who said so. Recorded, not validated: this is provenance for a reader,
    /// not an authorisation check.
    pub adjudicator: String,
    /// Optional free-text note. Bounded, and never parsed.
    pub note: String,
}

/// Most characters of an adjudicator's note that reach the row.
const MAX_NOTE_CHARS: usize = 240;

/// Record one verdict.
pub fn record_adjudication(
    db: &DbInstance,
    ledger_dir: &std::path::Path,
    pending: &PendingAttribution,
    verdict: &AttributionVerdict,
    adjudicated_at: DateTime<Utc>,
) -> Result<MetricWriteOutcome, CognitiveError> {
    if verdict.adjudicator.trim().is_empty() {
        return Err(CognitiveError::Metric(
            "an adjudication must name its adjudicator".into(),
        ));
    }
    let store = MetricEventStore::new(db, ledger_dir)?;
    let window = attribution_window(adjudicated_at);
    store.declare_window(&window)?;
    store.record(&adjudication_event(
        pending,
        verdict,
        &window,
        adjudicated_at,
    ))
}

fn adjudication_event(
    pending: &PendingAttribution,
    verdict: &AttributionVerdict,
    window: &crate::metrics::window::EvaluationWindow,
    adjudicated_at: DateTime<Utc>,
) -> CognitiveMetricEvent {
    let adjudicated = verdict
        .adjudicated_candidate_id
        .clone()
        .unwrap_or_else(|| pending.no_cause_verdict());
    let accepted = pending.attribution_cohort == "accepted";
    let mut event = CognitiveMetricEvent::new(
        format!(
            "causal-attribution-adjudicated:{CAUSAL_ATTRIBUTION_VERSION}:{}",
            pending.correction_id
        ),
        ADJUDICATION_METRIC,
        MetricEventKind::AttributionEvaluated,
        window.evaluation_window_id.clone(),
        pending.cohort.clone(),
        adjudicated_at,
    )
    .with_session(pending.session_id.clone(), pending.turn_number)
    .with_outcome("adjudicated")
    .with_identity("correction_id", pending.correction_id.clone())
    .with_identity("decision_id", pending.decision_id.clone())
    .with_identity("action_attempt_id", pending.action_attempt_id.clone())
    // The engine's proposal, carried unchanged. Precision is the equality of
    // this and the next field, so both have to live on one row.
    .with_identity("causal_candidate_id", pending.proposed_candidate_id.clone())
    .with_identity("adjudicated_causal_candidate_id", adjudicated)
    .with_identity("attribution_adjudication_id", pending.adjudication_id())
    .with_identity("cause_action_class", pending.cause_action_class.clone())
    .with_identity("accepted", if accepted { "true" } else { "false" })
    .with_identity(
        "abstained",
        if pending.attribution_cohort == "abstained" {
            "true"
        } else {
            "false"
        },
    )
    .with_identity("attribution_cohort", pending.attribution_cohort.clone())
    // The filter `causal_attribution_precision` selects on. Only rows whose
    // engine verdict was an acceptance are accepted-link precision.
    .with_identity(ADJUDICATION_SCOPE, pending.attribution_cohort.clone())
    .with_identity("attribution_version", CAUSAL_ATTRIBUTION_VERSION)
    .with_identity("adjudicator", verdict.adjudicator.trim().to_string())
    .with_identity("lesson_id", pending.lesson_id.clone())
    .with_identity("mutation_source", "none");
    event.evidence_refs = vec![
        format!("correction:{}", pending.correction_id),
        format!("session:{}", pending.session_id),
        format!("turn:{}", pending.turn_number),
        format!("adjudicator:{}", verdict.adjudicator.trim()),
    ];
    let note: String = verdict.note.chars().take(MAX_NOTE_CHARS).collect();
    if !note.trim().is_empty() {
        event.evidence_refs.push(format!("note:{note}"));
    }
    event.label_source = ADJUDICATION_LABEL_SOURCE.to_string();
    event
}

#[cfg(test)]
#[path = "adjudication/tests.rs"]
mod tests;
