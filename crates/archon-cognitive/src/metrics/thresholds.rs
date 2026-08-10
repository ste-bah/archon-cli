//! Declared release thresholds, versioned as code.
//!
//! Same rule as [`crate::metrics::definitions`]: a threshold is not a mutable
//! row. Loosening a bound to get a release out must be a reviewable diff that
//! bumps [`METRIC_THRESHOLD_VERSION`], not an `UPDATE` nobody sees, because a
//! gate whose bounds can be edited underneath a closed evaluation window is not
//! a gate.
//!
//! Every threshold names the metric-definition version it was calibrated
//! against. A bound reasoned about under one formula says nothing about a
//! different formula, so the two versions travel together and
//! [`thresholds_match_definition_version`] refuses to let them drift apart.

use crate::metrics::definitions::{METRIC_DEFINITION_VERSION, metric_definitions};

/// Version of the threshold table below. Change a bound, change this.
pub const METRIC_THRESHOLD_VERSION: i64 = 1;

/// Which side of the bound a healthy value sits on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThresholdBound {
    /// Healthy at or above `f64` (coverage, agreement, acceptance).
    AtLeast(f64),
    /// Healthy at or below `f64` (error, staleness, abstention).
    AtMost(f64),
}

impl ThresholdBound {
    pub fn admits(self, observed: f64) -> bool {
        match self {
            Self::AtLeast(floor) => observed >= floor,
            Self::AtMost(ceiling) => observed <= ceiling,
        }
    }

    pub fn describe(self) -> String {
        match self {
            Self::AtLeast(floor) => format!(">= {floor:.4}"),
            Self::AtMost(ceiling) => format!("<= {ceiling:.4}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricThreshold {
    /// Must name an entry in [`metric_definitions`].
    pub metric_name: &'static str,
    /// Metric-definition version this bound was reasoned about under.
    pub calibrated_for_definition_version: i64,
    pub bound: ThresholdBound,
    /// Below this many eligible events the segment is reported as having
    /// insufficient evidence rather than being judged. A promotion must not be
    /// blocked — or waved through — on three observations.
    pub min_sample_count: usize,
    /// Why this number and not another one. Read by the operator staring at a
    /// blocked release, so it has to justify the bound, not restate it.
    pub rationale: &'static str,
}

/// Version-1 thresholds.
///
/// Deliberately small. Each bound below has an argument for its exact value;
/// a metric with no defensible bound is left ungated rather than given a
/// round number that would block releases for no stated reason.
pub fn metric_thresholds() -> &'static [MetricThreshold] {
    const THRESHOLDS: &[MetricThreshold] = &[
        MetricThreshold {
            metric_name: "verified_success_label_coverage",
            calibrated_for_definition_version: METRIC_DEFINITION_VERSION,
            bound: ThresholdBound::AtLeast(0.5),
            min_sample_count: 20,
            rationale: "Below half deterministic labels, every other metric in \
                        this cohort is computed on a minority of its outcomes \
                        and cannot carry a promotion decision.",
        },
        MetricThreshold {
            metric_name: "self_model_confidence_calibration_error",
            calibrated_for_definition_version: METRIC_DEFINITION_VERSION,
            bound: ThresholdBound::AtMost(0.25),
            min_sample_count: 20,
            rationale: "0.25 is the Brier score of the constant 0.5 predictor. \
                        Worse than that means the self-model's confidence is \
                        less useful than declining to predict.",
        },
        MetricThreshold {
            metric_name: "stale_rule_prompt_share",
            calibrated_for_definition_version: METRIC_DEFINITION_VERSION,
            bound: ThresholdBound::AtMost(0.25),
            min_sample_count: 20,
            rationale: "Above a quarter of injected rules being stale, the \
                        prompt is largely describing a rule set that no longer \
                        exists, so behaviour attributed to the new rules is \
                        not attributable.",
        },
        MetricThreshold {
            metric_name: "correction_classifier_abstention_rate",
            calibrated_for_definition_version: METRIC_DEFINITION_VERSION,
            bound: ThresholdBound::AtMost(0.5),
            min_sample_count: 20,
            rationale: "A classifier abstaining on more than half its inputs is \
                        not measuring corrections; the correction-derived \
                        metrics beneath it describe the minority it did judge.",
        },
    ];
    THRESHOLDS
}

/// Threshold metric names with no matching entry in [`metric_definitions`].
///
/// A typo here would silently disable a gate — the lookup would simply never
/// find a metric to judge — so the mismatch is surfaced as data and asserted
/// empty by the threshold tests.
pub fn unknown_threshold_metrics() -> Vec<&'static str> {
    metric_thresholds()
        .iter()
        .map(|threshold| threshold.metric_name)
        .filter(|name| {
            !metric_definitions()
                .iter()
                .any(|definition| definition.name == *name)
        })
        .collect()
}

/// Whether every threshold was calibrated against the live definition version.
///
/// False means someone changed a formula without revisiting the bounds derived
/// from it. The gate treats such a check as unjudgeable rather than passing it.
pub fn thresholds_match_definition_version() -> bool {
    metric_thresholds()
        .iter()
        .all(|threshold| threshold.calibrated_for_definition_version == METRIC_DEFINITION_VERSION)
}
