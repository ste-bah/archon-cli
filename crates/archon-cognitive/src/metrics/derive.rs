//! Derivation of metric values from raw events over an evaluation window.
//!
//! Nothing here reads a counter. Every number is recomputed from the
//! append-only event rows, so replaying the same window always yields the
//! same snapshot and a definition change never needs a data migration.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::metrics::definitions::{
    METRIC_DEFINITION_VERSION, MetricAggregation, MetricDefinition, VERIFIED_FAILED,
    VERIFIED_PASSED, metric_definitions,
};
use crate::metrics::event::CognitiveMetricEvent;
use crate::metrics::window::{EvaluationWindow, MetricCohort};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DerivedMetric {
    pub metric_name: String,
    pub metric_definition_version: i64,
    pub cohort: MetricCohort,
    /// `None` where the definition has no defined value for the population
    /// it was given (empty numeric column, zero denominator, no verified
    /// outcomes). Never coerced to zero.
    pub value: Option<f64>,
    pub numerator: f64,
    pub denominator: f64,
    pub sample_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CognitiveMetricSnapshot {
    pub metric_definition_version: i64,
    pub evaluation_window: Option<EvaluationWindow>,
    pub event_count: usize,
    pub cohort_count: usize,
    pub metrics: Vec<DerivedMetric>,
}

impl CognitiveMetricSnapshot {
    pub fn empty() -> Self {
        Self {
            metric_definition_version: METRIC_DEFINITION_VERSION,
            evaluation_window: None,
            event_count: 0,
            cohort_count: 0,
            metrics: Vec::new(),
        }
    }

    pub fn metric(&self, name: &str, cohort: &MetricCohort) -> Option<&DerivedMetric> {
        self.metrics
            .iter()
            .find(|metric| metric.metric_name == name && &metric.cohort == cohort)
    }

    pub fn pooled(&self, name: &str) -> Option<&DerivedMetric> {
        self.metric(name, &MetricCohort::pooled())
    }
}

/// Recompute every version-1 metric for `window` from `events`.
///
/// With no window the whole event history is the population, which is what
/// the inspection surface falls back to before any window is declared.
pub fn derive_snapshot(
    window: Option<&EvaluationWindow>,
    events: &[CognitiveMetricEvent],
) -> CognitiveMetricSnapshot {
    let eligible: Vec<&CognitiveMetricEvent> = events
        .iter()
        .filter(|event| is_eligible(window, event))
        .collect();

    // The pooled cohort is reported *alongside* the segments so a promotion
    // decision can never be made on the aggregate alone.
    let mut cohorts: BTreeSet<MetricCohort> =
        eligible.iter().map(|event| event.cohort.clone()).collect();
    let cohort_count = cohorts.len();
    cohorts.insert(MetricCohort::pooled());

    let mut metrics = Vec::new();
    for cohort in &cohorts {
        let cohort_events: Vec<&CognitiveMetricEvent> = if cohort.is_pooled() {
            eligible.clone()
        } else {
            eligible
                .iter()
                .filter(|event| &event.cohort == cohort)
                .copied()
                .collect()
        };
        for definition in metric_definitions() {
            if let Some(metric) = derive_one(definition, cohort, &cohort_events) {
                metrics.push(metric);
            }
        }
    }
    metrics.sort_by(|left, right| {
        left.metric_name
            .cmp(&right.metric_name)
            .then_with(|| left.cohort.cmp(&right.cohort))
    });

    CognitiveMetricSnapshot {
        metric_definition_version: METRIC_DEFINITION_VERSION,
        evaluation_window: window.cloned(),
        event_count: eligible.len(),
        cohort_count,
        metrics,
    }
}

/// A window owns both an identity and immutable bounds; an event must satisfy
/// both to enter the population, so neither a mislabelled event nor a
/// late-arriving one can quietly change a closed window.
fn is_eligible(window: Option<&EvaluationWindow>, event: &CognitiveMetricEvent) -> bool {
    match window {
        None => true,
        Some(window) => {
            event.evaluation_window_id == window.evaluation_window_id
                && window.contains(event.created_at)
        }
    }
}

fn derive_one(
    definition: &MetricDefinition,
    cohort: &MetricCohort,
    cohort_events: &[&CognitiveMetricEvent],
) -> Option<DerivedMetric> {
    let matching: Vec<&CognitiveMetricEvent> = cohort_events
        .iter()
        .filter(|event| event.event_kind == definition.event_kind)
        .filter(|event| match definition.identity_filter {
            None => true,
            Some((key, expected)) => event.identity(key) == Some(expected),
        })
        .copied()
        .collect();
    if matching.is_empty() {
        return None;
    }

    let (value, numerator, denominator) =
        aggregate(definition.aggregation, &matching, cohort_events);
    Some(DerivedMetric {
        metric_name: definition.name.to_string(),
        metric_definition_version: definition.version,
        cohort: cohort.clone(),
        value,
        numerator,
        denominator,
        sample_count: matching.len(),
    })
}

fn aggregate(
    aggregation: MetricAggregation,
    matching: &[&CognitiveMetricEvent],
    cohort_events: &[&CognitiveMetricEvent],
) -> (Option<f64>, f64, f64) {
    match aggregation {
        MetricAggregation::Count => {
            let count = matching.len() as f64;
            (Some(count), count, 1.0)
        }
        MetricAggregation::Mean => {
            let values = numeric_values(matching);
            ratio(values.iter().sum(), values.len() as f64, None)
        }
        MetricAggregation::Percentile { percentile } => {
            let mut values = numeric_values(matching);
            values.sort_by(f64::total_cmp);
            let denominator = values.len() as f64;
            match nearest_rank(&values, percentile) {
                Some(value) => (Some(value), value, denominator),
                None => (None, 0.0, denominator),
            }
        }
        MetricAggregation::PooledRatio {
            zero_denominator_value,
        } => {
            let numerator: f64 = matching
                .iter()
                .map(|event| event.numerator.unwrap_or(0.0))
                .sum();
            let denominator: f64 = matching
                .iter()
                .map(|event| event.denominator.unwrap_or(0.0))
                .sum();
            ratio(numerator, denominator, zero_denominator_value)
        }
        MetricAggregation::IdentityRate { key, positive } => {
            let numerator = matching
                .iter()
                .filter(|event| event.identity(key) == Some(positive))
                .count() as f64;
            ratio(numerator, matching.len() as f64, None)
        }
        MetricAggregation::IdentityMatchRate { left, right } => {
            let numerator = matching
                .iter()
                .filter(
                    |event| match (event.identity(left), event.identity(right)) {
                        // A row missing either side is counted in the denominator
                        // and not the numerator. Dropping it would quietly restrict
                        // the population to the rows that happen to agree.
                        (Some(left_value), Some(right_value)) => left_value == right_value,
                        _ => false,
                    },
                )
                .count() as f64;
            ratio(numerator, matching.len() as f64, None)
        }
        MetricAggregation::OutcomeRate { positive } => {
            let numerator = matching
                .iter()
                .filter(|event| positive.contains(&event.outcome_status.as_str()))
                .count() as f64;
            ratio(numerator, matching.len() as f64, None)
        }
        MetricAggregation::BrierScore => {
            let squared: Vec<f64> = matching
                .iter()
                .filter_map(|event| {
                    let predicted = event.value?;
                    let observed = binary_outcome(&event.outcome_status)?;
                    Some((predicted - observed).powi(2))
                })
                .collect();
            ratio(squared.iter().sum(), squared.len() as f64, None)
        }
        MetricAggregation::RatePer100Turns => {
            let turns: BTreeSet<(&str, u64)> = cohort_events
                .iter()
                .map(|event| (event.session_id.as_str(), event.turn_number))
                .collect();
            ratio(matching.len() as f64 * 100.0, turns.len() as f64, None)
        }
    }
}

fn ratio(
    numerator: f64,
    denominator: f64,
    zero_denominator_value: Option<f64>,
) -> (Option<f64>, f64, f64) {
    if denominator == 0.0 {
        return (zero_denominator_value, numerator, denominator);
    }
    (Some(numerator / denominator), numerator, denominator)
}

fn numeric_values(events: &[&CognitiveMetricEvent]) -> Vec<f64> {
    events.iter().filter_map(|event| event.value).collect()
}

/// Nearest-rank percentile on already-sorted values: no interpolation, so the
/// result is exactly one observed sample and is stable across platforms.
fn nearest_rank(sorted: &[f64], percentile: u8) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (f64::from(percentile) / 100.0 * sorted.len() as f64).ceil() as usize;
    sorted.get(rank.clamp(1, sorted.len()) - 1).copied()
}

fn binary_outcome(outcome_status: &str) -> Option<f64> {
    match outcome_status {
        VERIFIED_PASSED => Some(1.0),
        VERIFIED_FAILED => Some(0.0),
        _ => None,
    }
}

/// Group derived metrics by cohort for surfaces that render one block per
/// segment rather than a flat list.
pub fn by_cohort(snapshot: &CognitiveMetricSnapshot) -> BTreeMap<String, Vec<&DerivedMetric>> {
    let mut grouped: BTreeMap<String, Vec<&DerivedMetric>> = BTreeMap::new();
    for metric in &snapshot.metrics {
        grouped
            .entry(metric.cohort.segmentation_key())
            .or_default()
            .push(metric);
    }
    grouped
}
