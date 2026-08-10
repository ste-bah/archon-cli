//! Versioned metric definitions.
//!
//! The roadmap requires metric definitions to be code, not mutable rows: a
//! threshold or formula change must produce a new version rather than
//! retroactively rewriting a closed evaluation window.

use crate::metrics::event::MetricEventKind;

/// Version of the definition table below. Change a formula, change this.
///
/// 2: added the shadow-executive and self-model-fact definitions, which had no
/// entries while nothing emitted their events.
///
/// 3: `lesson_citation_rate` now counts the `cited` identity rather than
/// `rule_injected`. Every emitter of `retrieval_hit_observed` records only hits
/// it actually injected, so the old key made the "citation rate" a constant
/// 1.0 — a number that reports nothing. Added `reflection_verified_reuse_rate`
/// alongside it, which is deliberately *not* the citation rate: reuse requires
/// the deterministic verification to have passed as well.
///
/// 4: added `governed_proposal_reversal_rate` and
/// `consolidated_memory_reuse_rate`, which became measurable once the memory
/// garden started raising governed proposals and writing consolidated memories.
/// No existing formula changed, so prior windows recompute identically; the
/// version moves because the definition TABLE did, which is what a reader
/// comparing two snapshots needs to know.
///
/// 5: added the three R2 attribution definitions the roadmap names and this
/// table could not previously express — `causal_attribution_precision` (which
/// needed a cross-field equality aggregation, since an accepted link is correct
/// only when the proposed and adjudicated candidate ids are equal) and the
/// accepted/comparator repeated-verified-failure rates over the new
/// `attribution_followup_evaluated` events. No existing formula changed.
pub const METRIC_DEFINITION_VERSION: i64 = 5;

/// How a metric collapses its eligible events into one number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MetricAggregation {
    /// Number of eligible events.
    Count,
    /// Mean of `value`.
    Mean,
    /// Nearest-rank percentile of `value`.
    Percentile { percentile: u8 },
    /// `sum(numerator) / sum(denominator)`.
    ///
    /// `zero_denominator_value` is `Some` only where the roadmap defines the
    /// empty case (the stale-rule share is defined as `0`); everywhere else a
    /// zero denominator yields no value rather than a fabricated one.
    PooledRatio { zero_denominator_value: Option<f64> },
    /// Fraction of eligible events whose identity `key` equals `positive`.
    IdentityRate {
        key: &'static str,
        positive: &'static str,
    },
    /// Fraction of eligible events whose identity `left` equals identity
    /// `right`.
    ///
    /// Exists for precision as the roadmap defines it: "an accepted link is
    /// correct only when `causal_candidate_id == adjudicated_causal_candidate_id`".
    /// That is a statement about two columns of the same row, which no
    /// single-key rate can express — and expressing it any other way would mean
    /// deciding correctness somewhere other than the metric definition.
    ///
    /// An event missing either identity is counted as not matching rather than
    /// dropped: a row that cannot say what it was adjudicated to is not a
    /// correct link.
    IdentityMatchRate {
        left: &'static str,
        right: &'static str,
    },
    /// Fraction of eligible events whose `outcome_status` is in `positive`.
    OutcomeRate { positive: &'static [&'static str] },
    /// Brier score `mean((predicted - y)^2)` over deterministically verified
    /// events; unknown/skipped outcomes are excluded, never coerced.
    BrierScore,
    /// Eligible events per 100 distinct turns observed in the same cohort.
    RatePer100Turns,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricDefinition {
    pub name: &'static str,
    pub version: i64,
    pub event_kind: MetricEventKind,
    /// Optional identity predicate narrowing the eligible population.
    pub identity_filter: Option<(&'static str, &'static str)>,
    pub aggregation: MetricAggregation,
}

/// Outcome statuses that count as a deterministic verified pass/fail.
pub const VERIFIED_PASSED: &str = "passed";
pub const VERIFIED_FAILED: &str = "failed";

const DETERMINISTIC_OUTCOMES: &[&str] = &[VERIFIED_PASSED, VERIFIED_FAILED];
const UNKNOWN_OUTCOMES: &[&str] = &["unknown"];

/// Version-1 definitions.
///
/// This is the deterministic subset of the roadmap metric list: every entry
/// here is computable from raw events alone, with no baseline comparison and
/// no interval estimation (both deferred).
pub fn metric_definitions() -> &'static [MetricDefinition] {
    const DEFINITIONS: &[MetricDefinition] = &[
        MetricDefinition {
            name: "corrections_per_100_turns",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::CorrectionClassified,
            identity_filter: Some(("ground_truth_label", "correction")),
            aggregation: MetricAggregation::RatePer100Turns,
        },
        MetricDefinition {
            name: "correction_classifier_abstention_rate",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::CorrectionClassified,
            identity_filter: None,
            aggregation: MetricAggregation::IdentityRate {
                key: "abstained",
                positive: "true",
            },
        },
        MetricDefinition {
            name: "causal_attribution_accept_rate",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::AttributionEvaluated,
            identity_filter: None,
            aggregation: MetricAggregation::IdentityRate {
                key: "accepted",
                positive: "true",
            },
        },
        MetricDefinition {
            name: "causal_attribution_abstention_rate",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::AttributionEvaluated,
            identity_filter: None,
            aggregation: MetricAggregation::IdentityRate {
                key: "abstained",
                positive: "true",
            },
        },
        // Only adjudicated rows are eligible, and only those whose adjudication
        // was about an accepted link. The shadow rows the engine writes carry no
        // `adjudication_scope`, so an unadjudicated corpus produces no value at
        // all rather than a precision of 1.0 computed against its own proposals.
        MetricDefinition {
            name: "causal_attribution_precision",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::AttributionEvaluated,
            identity_filter: Some(("adjudication_scope", "accepted")),
            aggregation: MetricAggregation::IdentityMatchRate {
                left: "causal_candidate_id",
                right: "adjudicated_causal_candidate_id",
            },
        },
        MetricDefinition {
            name: "causal_attribution_repeated_verified_failure_rate",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::AttributionFollowupEvaluated,
            identity_filter: None,
            aggregation: MetricAggregation::OutcomeRate {
                positive: &[VERIFIED_FAILED],
            },
        },
        // The promotion comparison is between these two, not between either and
        // a bound. They are separate definitions because the eligible population
        // differs; the relative reduction between them is computed by whoever
        // reads the snapshot, over matched strata.
        MetricDefinition {
            name: "causal_attribution_accepted_repeated_verified_failure_rate",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::AttributionFollowupEvaluated,
            identity_filter: Some(("attribution_cohort", "accepted")),
            aggregation: MetricAggregation::OutcomeRate {
                positive: &[VERIFIED_FAILED],
            },
        },
        MetricDefinition {
            name: "causal_attribution_comparator_repeated_verified_failure_rate",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::AttributionFollowupEvaluated,
            // Abstained and unattributed pooled, which is what the roadmap's
            // comparator is. Carried as one identity because a definition can
            // filter on one key, and an "is not accepted" predicate written any
            // other way would silently include a cohort added later.
            identity_filter: Some(("followup_comparator", "true")),
            aggregation: MetricAggregation::OutcomeRate {
                positive: &[VERIFIED_FAILED],
            },
        },
        MetricDefinition {
            name: "rule_create_count",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::RuleLifecycleObserved,
            identity_filter: Some(("rule_operation", "create")),
            aggregation: MetricAggregation::Count,
        },
        MetricDefinition {
            name: "rule_reinforce_count",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::RuleLifecycleObserved,
            identity_filter: Some(("rule_operation", "reinforce")),
            aggregation: MetricAggregation::Count,
        },
        MetricDefinition {
            name: "rule_retire_count",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::RuleLifecycleObserved,
            identity_filter: Some(("rule_operation", "retire")),
            aggregation: MetricAggregation::Count,
        },
        MetricDefinition {
            name: "governed_proposal_acceptance_rate",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::GovernedProposalObserved,
            identity_filter: Some(("proposal_lifecycle_operation", "decide")),
            aggregation: MetricAggregation::IdentityRate {
                key: "proposal_decision",
                positive: "accepted",
            },
        },
        // How often an APPLIED governed change had to be undone. The roadmap's
        // R4 gate is stated over reversals among applied proposals, so the
        // population is applications rather than decisions.
        //
        // A pooled ratio rather than an identity rate, because the two halves
        // are recorded at different moments and a metric event is immutable: an
        // apply cannot know whether it will later be rolled back. So an apply
        // contributes `denominator = 1`, a rollback contributes
        // `numerator = 1`, and the ratio is rollbacks over applications. The
        // identity filter keeps `decide` events — which carry neither — out of
        // the population entirely rather than relying on them summing to zero.
        MetricDefinition {
            name: "governed_proposal_reversal_rate",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::GovernedProposalObserved,
            identity_filter: Some(("proposal_application_outcome", "recorded")),
            aggregation: MetricAggregation::PooledRatio {
                zero_denominator_value: None,
            },
        },
        // Whether consolidated semantic memories are actually recalled. A
        // consolidation that is never read is prompt budget spent on tidiness,
        // and is the outcome the R4 gate calls semantic reuse.
        MetricDefinition {
            name: "consolidated_memory_reuse_rate",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::RetrievalHitObserved,
            identity_filter: Some(("consolidated_memory", "true")),
            aggregation: MetricAggregation::IdentityRate {
                key: "cited",
                positive: "true",
            },
        },
        MetricDefinition {
            name: "stale_rule_prompt_share",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::PromptRulesComposed,
            identity_filter: None,
            aggregation: MetricAggregation::PooledRatio {
                zero_denominator_value: Some(0.0),
            },
        },
        // Share of injected lessons the receiving turn cited. A citation says
        // the lesson was referenced and nothing more, which is why the reuse
        // metric below is a separate number rather than this one renamed.
        MetricDefinition {
            name: "lesson_citation_rate",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::RetrievalHitObserved,
            identity_filter: Some(("rule_injected", "true")),
            aggregation: MetricAggregation::IdentityRate {
                key: "cited",
                positive: "true",
            },
        },
        // Share of injected lessons that were cited *and* followed by a
        // deterministic verified pass. The promotion gate in the roadmap
        // (W6/R6) is stated over this, not over citations.
        MetricDefinition {
            name: "reflection_verified_reuse_rate",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::RetrievalHitObserved,
            identity_filter: Some(("rule_injected", "true")),
            aggregation: MetricAggregation::IdentityRate {
                key: "verified_reuse",
                positive: "true",
            },
        },
        MetricDefinition {
            name: "self_model_confidence_calibration_error",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::SelfModelPredictionEvaluated,
            identity_filter: None,
            aggregation: MetricAggregation::BrierScore,
        },
        MetricDefinition {
            name: "verified_success_label_coverage",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::WorldLabelMaterialized,
            identity_filter: None,
            aggregation: MetricAggregation::OutcomeRate {
                positive: DETERMINISTIC_OUTCOMES,
            },
        },
        MetricDefinition {
            name: "label_unknown_rate",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::WorldLabelMaterialized,
            identity_filter: None,
            aggregation: MetricAggregation::OutcomeRate {
                positive: UNKNOWN_OUTCOMES,
            },
        },
        // Shadow executive loop. The agreement rate is the whole point of
        // running a planner nobody executes: it says how often the loop would
        // have chosen what the live agent actually did. It is deliberately not
        // a success measure — a no-op executor never succeeds at anything.
        MetricDefinition {
            name: "shadow_action_agreement_rate",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::ShadowDecisionCompared,
            identity_filter: None,
            aggregation: MetricAggregation::IdentityRate {
                key: "agreed",
                positive: "true",
            },
        },
        // Surprise measured against the live turn rather than a model, so it
        // exists before any world model is validated. Mean and p95 together:
        // a rising tail is the signal a mean hides.
        MetricDefinition {
            name: "shadow_surprise_mean",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::ShadowDecisionCompared,
            identity_filter: None,
            aggregation: MetricAggregation::Mean,
        },
        MetricDefinition {
            name: "shadow_surprise_p95",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::ShadowDecisionCompared,
            identity_filter: None,
            aggregation: MetricAggregation::Percentile { percentile: 95 },
        },
        // Self-model writes. `value` is the fact's post-update confidence, so
        // the mean says where the self-model currently sits rather than how
        // often it was touched; the count says the latter.
        MetricDefinition {
            name: "self_model_fact_update_count",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::SelfModelFactUpdated,
            identity_filter: None,
            aggregation: MetricAggregation::Count,
        },
        MetricDefinition {
            name: "self_model_fact_confidence_mean",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::SelfModelFactUpdated,
            identity_filter: None,
            aggregation: MetricAggregation::Mean,
        },
        MetricDefinition {
            name: "latent_surprise_mean",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::SurpriseObserved,
            identity_filter: None,
            aggregation: MetricAggregation::Mean,
        },
        MetricDefinition {
            name: "latent_surprise_p95",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::SurpriseObserved,
            identity_filter: None,
            aggregation: MetricAggregation::Percentile { percentile: 95 },
        },
    ];
    DEFINITIONS
}
