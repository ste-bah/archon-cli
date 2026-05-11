use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::{CpuLatentTransitionModel, LatentTransitionExample};
use crate::schema::WorldLabelSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalConfig {
    pub bootstrap_iterations: usize,
    pub confidence_level: f32,
    pub parity_precision: String,
    pub parity_min_cosine: f32,
    pub next_state_baseline_min_delta: f32,
    pub counterfactual_baseline_min_delta: f32,
    pub surprise_ks_min_p: f32,
    pub counterfactual_ndcg_min: f32,
}

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            bootstrap_iterations: 1_000,
            confidence_level: 0.95,
            parity_precision: "fp32".into(),
            parity_min_cosine: 0.95,
            next_state_baseline_min_delta: 0.10,
            counterfactual_baseline_min_delta: 0.10,
            surprise_ks_min_p: 0.05,
            counterfactual_ndcg_min: 0.60,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PromotionGateReport {
    pub cosine_error_improved: bool,
    pub surprise_ks_passed: bool,
    pub counterfactual_ndcg_passed: bool,
    pub brier_improved: bool,
    pub no_critical_regression: bool,
}

impl Default for PromotionGateReport {
    fn default() -> Self {
        Self {
            cosine_error_improved: false,
            surprise_ks_passed: false,
            counterfactual_ndcg_passed: false,
            brier_improved: false,
            no_critical_regression: false,
        }
    }
}

impl PromotionGateReport {
    pub fn all_primary_gates_passed(&self) -> bool {
        self.cosine_error_improved
            && self.surprise_ks_passed
            && self.counterfactual_ndcg_passed
            && self.brier_improved
            && self.no_critical_regression
    }

    pub fn from_component_gates(
        cosine: &NextStateCosineGateReport,
        surprise: &SurpriseKsReport,
        counterfactual_ndcg_passed: bool,
        brier: &BrierImprovementReport,
        no_critical_regression: bool,
    ) -> Self {
        Self {
            cosine_error_improved: cosine.passed,
            surprise_ks_passed: surprise.passed,
            counterfactual_ndcg_passed,
            brier_improved: brier.passed,
            no_critical_regression,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextStateCosineGateReport {
    pub candidate_mean_error: f32,
    pub nearest_neighbor_mean_error: f32,
    pub relative_improvement: f32,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurpriseKsReport {
    pub statistic: f32,
    pub p_value: f32,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrierImprovementReport {
    pub labels_improved: usize,
    pub total_labels: usize,
    pub min_delta: f32,
    pub min_labels: usize,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelBrierScore {
    pub label: String,
    pub candidate_brier: f32,
    pub baseline_brier: f32,
}

pub fn evaluate_next_state_cosine_gate(
    model: &CpuLatentTransitionModel,
    baseline_examples: &[LatentTransitionExample],
    heldout: &[LatentTransitionExample],
    min_relative_improvement: f32,
) -> Result<NextStateCosineGateReport> {
    if baseline_examples.is_empty() || heldout.is_empty() {
        bail!("cosine gate requires baseline and heldout examples");
    }

    let candidate_mean_error = model.mean_cosine_error(heldout)?;
    let nearest_neighbor_mean_error =
        nearest_neighbor_mean_cosine_error(baseline_examples, heldout)?;
    let relative_improvement = if nearest_neighbor_mean_error <= f32::EPSILON {
        0.0
    } else {
        (nearest_neighbor_mean_error - candidate_mean_error) / nearest_neighbor_mean_error
    };

    Ok(NextStateCosineGateReport {
        candidate_mean_error,
        nearest_neighbor_mean_error,
        relative_improvement,
        passed: relative_improvement >= min_relative_improvement,
    })
}

pub fn nearest_neighbor_mean_cosine_error(
    baseline_examples: &[LatentTransitionExample],
    heldout: &[LatentTransitionExample],
) -> Result<f32> {
    if baseline_examples.is_empty() || heldout.is_empty() {
        bail!("nearest-neighbor baseline requires non-empty examples");
    }

    let mut total = 0.0;
    for example in heldout {
        let neighbor = nearest_neighbor(baseline_examples, example)?;
        total += 1.0 - cosine_similarity(&neighbor.next_state, &example.next_state)?;
    }

    Ok(total / heldout.len() as f32)
}

pub fn evaluate_surprise_ks_gate(
    candidate_distances: &[f32],
    heldout_distances: &[f32],
    min_p_value: f32,
) -> Result<SurpriseKsReport> {
    if candidate_distances.is_empty() || heldout_distances.is_empty() {
        bail!("surprise KS gate requires non-empty distributions");
    }

    let statistic = two_sample_ks_statistic(candidate_distances, heldout_distances);
    let p_value = approximate_ks_p_value(
        statistic,
        candidate_distances.len(),
        heldout_distances.len(),
    );

    Ok(SurpriseKsReport {
        statistic,
        p_value,
        passed: p_value > min_p_value,
    })
}

pub fn evaluate_brier_improvement(
    scores: &[LabelBrierScore],
    min_delta: f32,
    min_labels: usize,
) -> BrierImprovementReport {
    let labels_improved = scores
        .iter()
        .filter(|score| score.candidate_brier + min_delta <= score.baseline_brier)
        .count();

    BrierImprovementReport {
        labels_improved,
        total_labels: scores.len(),
        min_delta,
        min_labels,
        passed: labels_improved >= min_labels,
    }
}

pub fn evaluate_auxiliary_label_brier(
    model: &CpuLatentTransitionModel,
    baseline_examples: &[LatentTransitionExample],
    heldout: &[LatentTransitionExample],
) -> Result<Vec<LabelBrierScore>> {
    let labels = model
        .auxiliary_heads
        .iter()
        .map(|head| head.label.clone())
        .collect::<Vec<_>>();
    labels
        .into_iter()
        .map(|label| {
            let baseline_probability = label_prevalence(baseline_examples, &label);
            let mut candidate_total = 0.0;
            let mut baseline_total = 0.0;
            for example in heldout {
                let actual = if label_value(&example.labels, &label) {
                    1.0
                } else {
                    0.0
                };
                let predicted = model
                    .predict_auxiliary(&example.state, &example.action)?
                    .into_iter()
                    .find(|prediction| prediction.label == label)
                    .map(|prediction| prediction.probability)
                    .unwrap_or(baseline_probability);
                candidate_total += (predicted - actual).powi(2);
                baseline_total += (baseline_probability - actual).powi(2);
            }
            Ok(LabelBrierScore {
                label,
                candidate_brier: candidate_total / heldout.len().max(1) as f32,
                baseline_brier: baseline_total / heldout.len().max(1) as f32,
            })
        })
        .collect()
}

fn nearest_neighbor<'a>(
    baseline_examples: &'a [LatentTransitionExample],
    example: &LatentTransitionExample,
) -> Result<&'a LatentTransitionExample> {
    let target = state_action_vector(example);
    baseline_examples
        .iter()
        .map(|candidate| {
            Ok((
                candidate,
                cosine_similarity(&target, &state_action_vector(candidate))?,
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(candidate, _)| candidate)
        .ok_or_else(|| anyhow::anyhow!("nearest-neighbor baseline is empty"))
}

fn state_action_vector(example: &LatentTransitionExample) -> Vec<f32> {
    let mut vector = Vec::with_capacity(example.state.len() + example.action.len());
    vector.extend_from_slice(&example.state);
    vector.extend_from_slice(&example.action);
    vector
}

fn label_prevalence(examples: &[LatentTransitionExample], label: &str) -> f32 {
    let positives = examples
        .iter()
        .filter(|example| label_value(&example.labels, label))
        .count() as f32;
    ((positives + 1.0) / (examples.len() as f32 + 2.0)).clamp(0.01, 0.99)
}

fn label_value(labels: &WorldLabelSet, label: &str) -> bool {
    match label {
        "failure" => labels.failure,
        "retry" => labels.retry,
        "provider_incident" => labels.provider_incident,
        "verification_needed" => labels.verification_needed,
        "user_correction" => labels.user_correction,
        "plan_drift" => labels.plan_drift,
        "high_cost" => labels.high_cost,
        "slow_run" => labels.slow_run,
        _ => false,
    }
}

fn two_sample_ks_statistic(left: &[f32], right: &[f32]) -> f32 {
    let mut left_sorted = left.to_vec();
    let mut right_sorted = right.to_vec();
    left_sorted.sort_by(f32::total_cmp);
    right_sorted.sort_by(f32::total_cmp);

    let mut left_idx = 0usize;
    let mut right_idx = 0usize;
    let mut max_delta = 0.0f32;

    while left_idx < left_sorted.len() || right_idx < right_sorted.len() {
        let next = match (left_sorted.get(left_idx), right_sorted.get(right_idx)) {
            (Some(left), Some(right)) => left.min(*right),
            (Some(left), None) => *left,
            (None, Some(right)) => *right,
            (None, None) => break,
        };

        while left_sorted
            .get(left_idx)
            .is_some_and(|value| *value <= next)
        {
            left_idx += 1;
        }
        while right_sorted
            .get(right_idx)
            .is_some_and(|value| *value <= next)
        {
            right_idx += 1;
        }

        let left_cdf = left_idx as f32 / left_sorted.len() as f32;
        let right_cdf = right_idx as f32 / right_sorted.len() as f32;
        max_delta = max_delta.max((left_cdf - right_cdf).abs());
    }

    max_delta
}

fn approximate_ks_p_value(statistic: f32, left_len: usize, right_len: usize) -> f32 {
    let effective_n = ((left_len * right_len) as f32 / (left_len + right_len) as f32).sqrt();
    let lambda = (effective_n + 0.12 + 0.11 / effective_n.max(0.000_001)) * statistic;
    (2.0 * (-2.0 * lambda * lambda).exp()).clamp(0.0, 1.0)
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> Result<f32> {
    if left.len() != right.len() {
        bail!("cosine inputs must have matching dimensions");
    }

    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        Ok(0.0)
    } else {
        Ok((dot / (left_norm * right_norm)).clamp(-1.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{CpuLatentTransitionModel, LatentTransitionExample};

    use super::*;

    fn example(state: [f32; 2], action: [f32; 2], next_state: [f32; 2]) -> LatentTransitionExample {
        LatentTransitionExample {
            state: state.to_vec(),
            action: action.to_vec(),
            next_state: next_state.to_vec(),
            labels: Default::default(),
        }
    }

    #[test]
    fn eval_defaults_match_prd_gates() {
        let cfg = EvalConfig::default();
        assert_eq!(cfg.bootstrap_iterations, 1_000);
        assert_eq!(cfg.confidence_level, 0.95);
        assert_eq!(cfg.parity_precision, "fp32");
        assert_eq!(cfg.parity_min_cosine, 0.95);
        assert_eq!(cfg.next_state_baseline_min_delta, 0.10);
        assert_eq!(cfg.counterfactual_ndcg_min, 0.60);
    }

    #[test]
    fn promotion_requires_every_primary_gate() {
        let mut report = PromotionGateReport {
            cosine_error_improved: true,
            surprise_ks_passed: true,
            counterfactual_ndcg_passed: true,
            brier_improved: true,
            no_critical_regression: false,
        };

        assert!(!report.all_primary_gates_passed());
        report.no_critical_regression = true;
        assert!(report.all_primary_gates_passed());
    }

    #[test]
    fn next_state_gate_requires_beating_nearest_neighbor_baseline() {
        let baseline = [
            example([0.0, 1.0], [0.0, 0.0], [1.0, 1.0]),
            example([10.0, 1.0], [0.0, 0.0], [11.0, 1.0]),
        ];
        let heldout = [example([5.0, 10.0], [0.0, 0.0], [6.0, 10.0])];
        let model = CpuLatentTransitionModel::fit(2, &baseline).unwrap();

        let report = evaluate_next_state_cosine_gate(&model, &baseline, &heldout, 0.10).unwrap();

        assert!(report.nearest_neighbor_mean_error > report.candidate_mean_error);
        assert!(report.relative_improvement >= 0.10);
        assert!(report.passed);
    }

    #[test]
    fn surprise_ks_gate_accepts_similar_distributions() {
        let report =
            evaluate_surprise_ks_gate(&[0.1, 0.2, 0.3, 0.4], &[0.1, 0.2, 0.3, 0.4], 0.05).unwrap();

        assert_eq!(report.statistic, 0.0);
        assert!(report.passed);
    }

    #[test]
    fn brier_gate_requires_three_label_improvements() {
        let scores = [
            LabelBrierScore {
                label: "failure".into(),
                candidate_brier: 0.10,
                baseline_brier: 0.13,
            },
            LabelBrierScore {
                label: "retry".into(),
                candidate_brier: 0.20,
                baseline_brier: 0.23,
            },
            LabelBrierScore {
                label: "plan_drift".into(),
                candidate_brier: 0.30,
                baseline_brier: 0.33,
            },
        ];

        let report = evaluate_brier_improvement(&scores, 0.02, 3);

        assert_eq!(report.labels_improved, 3);
        assert!(report.passed);
    }

    #[test]
    fn component_gates_feed_promotion_report() {
        let cosine = NextStateCosineGateReport {
            candidate_mean_error: 0.1,
            nearest_neighbor_mean_error: 0.2,
            relative_improvement: 0.5,
            passed: true,
        };
        let surprise = SurpriseKsReport {
            statistic: 0.0,
            p_value: 1.0,
            passed: true,
        };
        let brier = BrierImprovementReport {
            labels_improved: 3,
            total_labels: 7,
            min_delta: 0.02,
            min_labels: 3,
            passed: true,
        };

        let report =
            PromotionGateReport::from_component_gates(&cosine, &surprise, true, &brier, true);

        assert!(report.all_primary_gates_passed());
    }
}
