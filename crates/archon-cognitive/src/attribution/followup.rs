//! `attribution_followup_evaluated`: did the correction happen again?
//!
//! The R2 promotion metric is not accuracy. It is whether corrections that were
//! successfully attributed are followed by fewer repeated verified failures than
//! corrections that were not — measured over the same follow-up window and the
//! same eligible repeated-opportunity query, matched by task class, cause/action
//! class, model, policy and cohort-entry calendar window (roadmap line 151).
//!
//! Version 1 makes three definitions, each of which is a choice and none of
//! which the roadmap fixes for us:
//!
//! * **An opportunity is one later turn in the same session that ran at least
//!   one tool.** Not one per tool run: a turn that ran nine tools is one chance
//!   for the mistake to recur, not nine, and counting per run would let a single
//!   tool-heavy turn dominate a cohort rate.
//! * **A verified failure is a tool result the provider marked an error.** That
//!   is the only deterministic outcome signal the transcript carries. Prose is
//!   not consulted.
//! * **The stratum's cause/action class is the OPPORTUNITY's, not the
//!   attribution's.** Every opportunity here is a tool-running turn, so the key
//!   is constant across cohorts — which is the point. Keying on the
//!   attribution's class instead would put accepted links (`tool_run`,
//!   `decision`) and unattributed corrections (`none`) in disjoint strata, and
//!   the roadmap's own eligibility rule would then report zero eligible strata
//!   forever. The attribution's class travels on the row as
//!   `attributed_cause_action_class` so a reader can still segment by it.

use chrono::{DateTime, Duration, NaiveDate, Utc};

/// Fixed origin for the cohort-entry buckets.
const EPOCH_DAY: NaiveDate = match NaiveDate::from_ymd_opt(1970, 1, 1) {
    Some(date) => date,
    None => unreachable!(),
};

use crate::attribution::AttributionCohort;
use crate::metrics::definitions::{VERIFIED_FAILED, VERIFIED_PASSED};
use crate::metrics::event::{CognitiveMetricEvent, MetricEventKind};
use crate::metrics::window::{EvaluationWindow, MetricCohort};

/// Identity of the follow-up procedure. Part of every id it mints.
pub const FOLLOWUP_VERSION: &str = "attribution-followup/v1";

/// Metric name carried by every follow-up row.
pub const FOLLOWUP_METRIC: &str = "causal_attribution_followup";

pub const FOLLOWUP_LABEL_SOURCE: &str = "transcript_verified_outcome";

/// How many turns after a correction still count as a repeated opportunity.
///
/// Immutable per correction once chosen: the window id embeds it, so widening
/// it produces a new window rather than retroactively enlarging a closed one.
pub const FOLLOWUP_WINDOW_TURNS: u64 = 20;

/// Width of the cohort-entry calendar bucket, in days.
pub const COHORT_ENTRY_BUCKET_DAYS: i64 = 7;

/// Most attributions one turn will evaluate opportunities for.
///
/// A long session accumulates corrections; without a bound one turn could emit
/// hundreds of rows. The most recent are kept, which are the ones still inside
/// their follow-up window.
pub const MAX_TRACKED_ATTRIBUTIONS: usize = 25;

/// A prior attribution, as read back from its own metric row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributedCorrection {
    pub correction_id: String,
    pub session_id: String,
    pub turn_number: u64,
    /// `accepted`, `abstained`, or `unattributed`.
    pub attribution_cohort: String,
    pub attributed_cause_action_class: String,
    pub causal_candidate_id: String,
    pub task_class: String,
    pub model_id: String,
    pub policy_version: String,
    pub recorded_at: DateTime<Utc>,
}

impl AttributedCorrection {
    /// Whether this correction belongs to the comparator arm.
    ///
    /// Recorded as an identity because a metric definition filters on one key,
    /// and "not accepted" expressed any other way would silently absorb a
    /// cohort added later.
    pub fn is_comparator(&self) -> bool {
        self.attribution_cohort != AttributionCohort::Accepted.as_code()
    }

    /// The immutable follow-up window this correction's opportunities fall in.
    pub fn followup_window_id(&self) -> String {
        format!(
            "fw:{FOLLOWUP_VERSION}:{}:{}:{FOLLOWUP_WINDOW_TURNS}",
            self.session_id, self.turn_number
        )
    }

    /// The calendar bucket the correction entered its cohort in.
    pub fn cohort_entry_window_id(&self) -> String {
        cohort_entry_window_id(self.recorded_at)
    }

    /// Whether `turn_number` is a repeated opportunity for this correction.
    pub fn covers(&self, session_id: &str, turn_number: u64) -> bool {
        session_id == self.session_id
            && turn_number > self.turn_number
            && turn_number - self.turn_number <= FOLLOWUP_WINDOW_TURNS
    }
}

/// Read the attributions out of a metric event population.
///
/// Takes already-read events rather than a store handle so the selection is
/// testable without one, and so the caller owns the cost of the read.
pub fn attributed_corrections(
    events: &[CognitiveMetricEvent],
    session_id: &str,
) -> Vec<AttributedCorrection> {
    let mut corrections: Vec<AttributedCorrection> = events
        .iter()
        .filter(|event| event.event_kind == MetricEventKind::AttributionEvaluated)
        .filter(|event| event.session_id == session_id)
        // Adjudication rows repeat a correction that already has a shadow row;
        // counting both would double every opportunity.
        .filter(|event| event.identity("adjudication_scope").is_none())
        .filter_map(|event| {
            Some(AttributedCorrection {
                correction_id: event.identity("correction_id")?.to_string(),
                session_id: event.session_id.clone(),
                turn_number: event.turn_number,
                attribution_cohort: event.identity("attribution_cohort")?.to_string(),
                attributed_cause_action_class: event.identity("cause_action_class")?.to_string(),
                causal_candidate_id: event.identity("causal_candidate_id")?.to_string(),
                task_class: event.cohort.task_class.clone(),
                model_id: event.cohort.model_id.clone(),
                policy_version: event.cohort.policy_version.clone(),
                recorded_at: event.created_at,
            })
        })
        .collect();
    corrections.sort_by(|left, right| {
        right
            .turn_number
            .cmp(&left.turn_number)
            .then_with(|| left.correction_id.cmp(&right.correction_id))
    });
    corrections.truncate(MAX_TRACKED_ATTRIBUTIONS);
    corrections
}

/// One later turn that gave the mistake a chance to recur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowupOpportunity {
    pub session_id: String,
    pub turn_number: u64,
    /// Deterministic: at least one tool result in this turn was an error.
    pub verified_failure: bool,
    pub observed_at: DateTime<Utc>,
}

/// The calendar bucket `at` falls in, as an immutable id.
pub fn cohort_entry_window_id(at: DateTime<Utc>) -> String {
    // Days since the Unix epoch, so the bucket boundaries are fixed points on
    // the calendar rather than relative to any observation.
    let days = at.date_naive().signed_duration_since(EPOCH_DAY).num_days();
    let bucket = days.div_euclid(COHORT_ENTRY_BUCKET_DAYS);
    format!("cew:{COHORT_ENTRY_BUCKET_DAYS}d:{bucket}")
}

/// Exact-match stratum, version 1.
///
/// Task class, cause/action class, model, policy version and the cohort-entry
/// bucket, in that order. A string rather than a hash so a reader can see why
/// two opportunities did or did not match.
pub fn match_stratum_id(
    task_class: &str,
    cause_action_class: &str,
    model_id: &str,
    policy_version: &str,
    cohort_entry_window_id: &str,
) -> String {
    format!(
        "fms:v1:{task_class}|{cause_action_class}|{model_id}|{policy_version}|{cohort_entry_window_id}"
    )
}

/// The UTC-day evaluation window an opportunity is counted in.
pub fn followup_window(observed_at: DateTime<Utc>) -> EvaluationWindow {
    let day = observed_at.date_naive();
    let started_at = day.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc();
    EvaluationWindow::new(
        format!("causal-attribution-followup-{day}"),
        started_at,
        started_at + Duration::days(1),
    )
}

/// Deterministic row identity: one opportunity per correction per turn.
pub fn followup_event_id(correction_id: &str, session_id: &str, turn_number: u64) -> String {
    format!("{FOLLOWUP_VERSION}:{correction_id}:{session_id}:{turn_number}")
}

/// Build the `attribution_followup_evaluated` row for one opportunity.
pub fn followup_event(
    attribution: &AttributedCorrection,
    opportunity: &FollowupOpportunity,
    cohort: MetricCohort,
    window: &EvaluationWindow,
) -> CognitiveMetricEvent {
    // The opportunity's own class. See the module note on why this, and not the
    // attribution's, is the stratum key.
    let cause_action_class = crate::attribution::CauseActionClass::ToolRun.as_code();
    let cohort_entry = attribution.cohort_entry_window_id();
    let opportunity_id = format!(
        "fo:{}:{}:{}",
        attribution.correction_id, opportunity.session_id, opportunity.turn_number
    );

    let mut event = CognitiveMetricEvent::new(
        followup_event_id(
            &attribution.correction_id,
            &opportunity.session_id,
            opportunity.turn_number,
        ),
        FOLLOWUP_METRIC,
        MetricEventKind::AttributionFollowupEvaluated,
        window.evaluation_window_id.clone(),
        cohort,
        opportunity.observed_at,
    )
    .with_session(opportunity.session_id.clone(), opportunity.turn_number)
    // Binary and deterministic. `passed` is not "the turn went well" -- it is
    // "no tool result in this turn was an error", which is the only outcome the
    // transcript can support.
    .with_outcome(if opportunity.verified_failure {
        VERIFIED_FAILED
    } else {
        VERIFIED_PASSED
    })
    .with_ratio(f64::from(u8::from(opportunity.verified_failure)), 1.0)
    .with_identity("correction_id", attribution.correction_id.clone())
    .with_identity("followup_opportunity_id", opportunity_id)
    .with_identity("followup_window_id", attribution.followup_window_id())
    .with_identity(
        "followup_match_stratum_id",
        match_stratum_id(
            &attribution.task_class,
            cause_action_class,
            &attribution.model_id,
            &attribution.policy_version,
            &cohort_entry,
        ),
    )
    .with_identity("cohort_entry_window_id", cohort_entry)
    .with_identity("attribution_cohort", attribution.attribution_cohort.clone())
    .with_identity("cause_action_class", cause_action_class)
    .with_identity(
        "attributed_cause_action_class",
        attribution.attributed_cause_action_class.clone(),
    )
    .with_identity(
        "causal_candidate_id",
        attribution.causal_candidate_id.clone(),
    )
    .with_identity(
        "followup_comparator",
        if attribution.is_comparator() {
            "true"
        } else {
            "false"
        },
    )
    .with_identity(
        "correction_turn_number",
        attribution.turn_number.to_string(),
    )
    .with_identity("followup_version", FOLLOWUP_VERSION);
    event.evidence_refs = vec![
        format!("correction:{}", attribution.correction_id),
        format!("session:{}", opportunity.session_id),
        format!("turn:{}", opportunity.turn_number),
        format!("attribution_turn:{}", attribution.turn_number),
    ];
    event.label_source = FOLLOWUP_LABEL_SOURCE.to_string();
    event
}
