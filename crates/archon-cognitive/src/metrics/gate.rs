//! Release gate over derived cognitive metrics.
//!
//! [`crate::metrics::derive`] states the rule this module exists to enforce:
//! the pooled cohort is reported *alongside* the segments so a promotion
//! decision can never be made on the aggregate alone. A gate that read only the
//! pooled figure would tick the box and defeat the reason the segments exist —
//! a cohort can be badly degraded while the pooled number, dominated by a large
//! healthy cohort, stays inside the bound.
//!
//! So: every cohort present in the snapshot is judged separately, and one
//! failing segment fails the whole gate. The pooled cohort is judged too, but
//! it is one voice among the segments rather than the only one.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::metrics::derive::{CognitiveMetricSnapshot, DerivedMetric};
use crate::metrics::thresholds::{METRIC_THRESHOLD_VERSION, MetricThreshold, metric_thresholds};
use crate::metrics::window::MetricCohort;

/// Result of one threshold applied to one cohort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateOutcome {
    Passed,
    Failed,
    /// Fewer eligible events than the threshold declares it needs, or a
    /// metric whose value is undefined for this population. Reported rather
    /// than rounded into a pass, because "we did not measure enough" is a
    /// different claim from "we measured and it was fine".
    InsufficientEvidence,
    /// The metric was derived under a different definition version than the
    /// threshold was calibrated against, so the bound does not describe this
    /// number. Blocks, because an incomparable check is not a pass.
    DefinitionDrift,
}

impl GateOutcome {
    /// Whether this outcome withholds promotion.
    pub fn blocks(self) -> bool {
        matches!(self, Self::Failed | Self::DefinitionDrift)
    }

    /// Whether the segment was actually judged either way.
    pub fn is_conclusive(self) -> bool {
        matches!(self, Self::Passed | Self::Failed)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::InsufficientEvidence => "insufficient_evidence",
            Self::DefinitionDrift => "definition_drift",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateCheck {
    pub metric_name: String,
    pub cohort: MetricCohort,
    /// Rendered bound, e.g. `>= 0.5000`.
    pub bound: String,
    /// `None` where the definition had no defined value for this population.
    pub observed: Option<f64>,
    pub sample_count: usize,
    pub min_sample_count: usize,
    pub metric_definition_version: i64,
    pub outcome: GateOutcome,
    pub rationale: String,
}

impl GateCheck {
    /// One line naming the metric, the failing segment, and both numbers.
    pub fn describe(&self) -> String {
        format!(
            "{}[{}] observed={} required{} n={} ({})",
            self.metric_name,
            self.cohort.segmentation_key(),
            self.observed
                .map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| "undefined".to_string()),
            self.bound,
            self.sample_count,
            self.outcome.as_str(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseGateVerdict {
    Passed,
    Failed,
    /// No threshold found a cohort with enough evidence to judge. Distinct from
    /// `Passed`: nothing was cleared, there was simply nothing to clear.
    NotEvaluated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReleaseGateReport {
    pub metric_threshold_version: i64,
    pub metric_definition_version: i64,
    pub evaluation_window_id: Option<String>,
    pub verdict: ReleaseGateVerdict,
    /// Distinct non-pooled cohorts that were actually judged.
    pub segments_evaluated: usize,
    /// Every check, including the passing and the unjudgeable ones, so a
    /// reader can see what the verdict rests on.
    pub checks: Vec<GateCheck>,
}

impl ReleaseGateReport {
    pub fn blocks_promotion(&self) -> bool {
        matches!(self.verdict, ReleaseGateVerdict::Failed)
    }

    pub fn blocking_checks(&self) -> impl Iterator<Item = &GateCheck> {
        self.checks.iter().filter(|check| check.outcome.blocks())
    }

    /// One line per blocking check, each naming its segment.
    pub fn failure_summary(&self) -> Vec<String> {
        self.blocking_checks().map(GateCheck::describe).collect()
    }

    /// Distinct segmentation keys that blocked, pooled included when it is the
    /// one that failed.
    pub fn failing_segments(&self) -> Vec<String> {
        self.blocking_checks()
            .map(|check| check.cohort.segmentation_key())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

/// Judge `snapshot` against the declared thresholds, per cohort.
///
/// Fails if any single segment fails, regardless of the pooled figure.
pub fn evaluate_release_gate(snapshot: &CognitiveMetricSnapshot) -> ReleaseGateReport {
    let cohorts: BTreeSet<MetricCohort> = snapshot
        .metrics
        .iter()
        .map(|metric| metric.cohort.clone())
        .collect();

    let mut checks = Vec::new();
    for cohort in &cohorts {
        for threshold in metric_thresholds() {
            if let Some(metric) = snapshot.metric(threshold.metric_name, cohort) {
                checks.push(check_one(threshold, metric));
            }
        }
    }

    let segments_evaluated = checks
        .iter()
        .filter(|check| !check.cohort.is_pooled() && check.outcome.is_conclusive())
        .map(|check| check.cohort.segmentation_key())
        .collect::<BTreeSet<_>>()
        .len();

    ReleaseGateReport {
        metric_threshold_version: METRIC_THRESHOLD_VERSION,
        metric_definition_version: snapshot.metric_definition_version,
        evaluation_window_id: snapshot
            .evaluation_window
            .as_ref()
            .map(|window| window.evaluation_window_id.clone()),
        verdict: verdict_of(&checks),
        segments_evaluated,
        checks,
    }
}

fn check_one(threshold: &MetricThreshold, metric: &DerivedMetric) -> GateCheck {
    let outcome = if metric.metric_definition_version != threshold.calibrated_for_definition_version
    {
        GateOutcome::DefinitionDrift
    } else if metric.sample_count < threshold.min_sample_count {
        GateOutcome::InsufficientEvidence
    } else {
        match metric.value {
            None => GateOutcome::InsufficientEvidence,
            Some(observed) if threshold.bound.admits(observed) => GateOutcome::Passed,
            Some(_) => GateOutcome::Failed,
        }
    };

    GateCheck {
        metric_name: metric.metric_name.clone(),
        cohort: metric.cohort.clone(),
        bound: threshold.bound.describe(),
        observed: metric.value,
        sample_count: metric.sample_count,
        min_sample_count: threshold.min_sample_count,
        metric_definition_version: metric.metric_definition_version,
        outcome,
        rationale: threshold.rationale.to_string(),
    }
}

fn verdict_of(checks: &[GateCheck]) -> ReleaseGateVerdict {
    if checks.iter().any(|check| check.outcome.blocks()) {
        ReleaseGateVerdict::Failed
    } else if checks
        .iter()
        .any(|check| check.outcome == GateOutcome::Passed)
    {
        ReleaseGateVerdict::Passed
    } else {
        ReleaseGateVerdict::NotEvaluated
    }
}
