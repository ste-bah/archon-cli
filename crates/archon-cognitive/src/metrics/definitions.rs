//! Versioned metric definitions.
//!
//! The roadmap requires metric definitions to be code, not mutable rows: a
//! threshold or formula change must produce a new version rather than
//! retroactively rewriting a closed evaluation window.

use crate::metrics::event::MetricEventKind;

/// Version of the definition table below. Change a formula, change this.
pub const METRIC_DEFINITION_VERSION: i64 = 1;

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
        MetricDefinition {
            name: "stale_rule_prompt_share",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::PromptRulesComposed,
            identity_filter: None,
            aggregation: MetricAggregation::PooledRatio {
                zero_denominator_value: Some(0.0),
            },
        },
        MetricDefinition {
            name: "lesson_citation_rate",
            version: METRIC_DEFINITION_VERSION,
            event_kind: MetricEventKind::RetrievalHitObserved,
            identity_filter: None,
            aggregation: MetricAggregation::IdentityRate {
                key: "rule_injected",
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
