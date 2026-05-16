use crate::backend::BackendKind;
use crate::schema::WorldLabelSet;
use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelPrecision {
    Fp32,
    Fp16,
    Int8,
}

impl Default for ModelPrecision {
    fn default() -> Self {
        Self::Fp32
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatentWorldModelMetadata {
    pub model_id: String,
    pub state_dim: usize,
    pub backend: BackendKind,
    pub precision: ModelPrecision,
    pub row_count: u64,
    pub parameter_count: u64,
    pub created_at: DateTime<Utc>,
}

impl LatentWorldModelMetadata {
    pub fn candidate(state_dim: usize, backend: BackendKind, row_count: u64) -> Self {
        Self {
            model_id: format!("world-model-candidate-{}", uuid::Uuid::new_v4()),
            state_dim,
            backend,
            precision: ModelPrecision::Fp32,
            row_count,
            parameter_count: 0,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatentTransitionExample {
    pub state: Vec<f32>,
    pub action: Vec<f32>,
    pub next_state: Vec<f32>,
    pub labels: WorldLabelSet,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CpuLatentTransitionModel {
    pub metadata: LatentWorldModelMetadata,
    pub state_weights: Vec<f32>,
    pub action_weights: Vec<f32>,
    pub transition_bias: Vec<f32>,
    pub mean_delta: Vec<f32>,
    pub auxiliary_heads: Vec<AuxiliaryHead>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuxiliaryHead {
    pub label: String,
    pub bias: f32,
    pub state_weights: Vec<f32>,
    pub action_weights: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuxiliaryPrediction {
    pub label: String,
    pub probability: f32,
}

impl CpuLatentTransitionModel {
    pub fn fit(state_dim: usize, examples: &[LatentTransitionExample]) -> Result<Self> {
        if examples.is_empty() {
            bail!("at least one transition example is required");
        }

        let mut mean_delta = vec![0.0; state_dim];
        let mut state_weights = vec![0.0; state_dim];
        let mut action_weights = vec![0.0; state_dim];
        let mut next_means = vec![0.0; state_dim];
        let mut state_means = vec![0.0; state_dim];
        let mut action_means = vec![0.0; state_dim];
        for example in examples {
            validate_example(state_dim, example)?;
            for idx in 0..state_dim {
                mean_delta[idx] += example.next_state[idx] - example.state[idx];
                next_means[idx] += example.next_state[idx];
                state_means[idx] += example.state[idx];
                action_means[idx] += example.action.get(idx).copied().unwrap_or_default();
            }
        }

        let denom = examples.len() as f32;
        for idx in 0..state_dim {
            mean_delta[idx] /= denom;
            next_means[idx] /= denom;
            state_means[idx] /= denom;
            action_means[idx] /= denom;
            state_weights[idx] =
                covariance_weight(examples, idx, state_means[idx], next_means[idx], true);
            action_weights[idx] =
                covariance_weight(examples, idx, action_means[idx], next_means[idx], false);
        }
        let transition_bias = (0..state_dim)
            .map(|idx| {
                next_means[idx]
                    - state_weights[idx] * state_means[idx]
                    - action_weights[idx] * action_means[idx]
            })
            .collect::<Vec<_>>();
        Self::from_fitted_transition_parts(
            state_dim,
            BackendKind::Cpu,
            examples.len() as u64,
            state_weights,
            action_weights,
            transition_bias,
            mean_delta,
            examples,
        )
    }

    pub(crate) fn from_fitted_transition_parts(
        state_dim: usize,
        backend: BackendKind,
        row_count: u64,
        state_weights: Vec<f32>,
        action_weights: Vec<f32>,
        transition_bias: Vec<f32>,
        mean_delta: Vec<f32>,
        examples: &[LatentTransitionExample],
    ) -> Result<Self> {
        if examples.is_empty() {
            bail!("at least one transition example is required");
        }
        if state_weights.len() != state_dim
            || action_weights.len() != state_dim
            || transition_bias.len() != state_dim
            || mean_delta.len() != state_dim
        {
            bail!("transition model parameter dimensions must match state_dim");
        }
        if state_weights.iter().any(|value| !value.is_finite())
            || action_weights.iter().any(|value| !value.is_finite())
            || transition_bias.iter().any(|value| !value.is_finite())
            || mean_delta.iter().any(|value| !value.is_finite())
        {
            bail!("transition model parameters must be finite");
        }
        for example in examples {
            validate_example(state_dim, example)?;
        }

        let auxiliary_heads = fit_auxiliary_heads(state_dim, examples);
        let parameter_count = (state_weights.len()
            + action_weights.len()
            + transition_bias.len()
            + auxiliary_heads
                .iter()
                .map(|head| 1 + head.state_weights.len() + head.action_weights.len())
                .sum::<usize>()) as u64;
        let mut metadata = LatentWorldModelMetadata::candidate(state_dim, backend, row_count);
        metadata.parameter_count = parameter_count;

        Ok(Self {
            metadata,
            state_weights,
            action_weights,
            transition_bias,
            mean_delta,
            auxiliary_heads,
        })
    }

    pub fn predict_next(&self, state: &[f32], action: &[f32]) -> Result<Vec<f32>> {
        if state.len() != self.metadata.state_dim {
            bail!(
                "state dimension mismatch: expected {}, got {}",
                self.metadata.state_dim,
                state.len()
            );
        }

        Ok((0..self.metadata.state_dim)
            .map(|idx| {
                let action_value = action.get(idx).copied().unwrap_or_default();
                self.transition_bias[idx]
                    + self.state_weights[idx] * state[idx]
                    + self.action_weights[idx] * action_value
            })
            .collect())
    }

    pub fn predict_auxiliary(
        &self,
        state: &[f32],
        action: &[f32],
    ) -> Result<Vec<AuxiliaryPrediction>> {
        if state.len() != self.metadata.state_dim {
            bail!(
                "state dimension mismatch: expected {}, got {}",
                self.metadata.state_dim,
                state.len()
            );
        }
        Ok(self
            .auxiliary_heads
            .iter()
            .map(|head| AuxiliaryPrediction {
                label: head.label.clone(),
                probability: sigmoid(
                    head.bias
                        + dot_prefix(&head.state_weights, state)
                        + dot_prefix(&head.action_weights, action),
                ),
            })
            .collect())
    }

    pub fn mean_cosine_error(&self, examples: &[LatentTransitionExample]) -> Result<f32> {
        let mut total = 0.0;
        for example in examples {
            validate_example(self.metadata.state_dim, example)?;
            let predicted = self.predict_next(&example.state, &example.action)?;
            total += 1.0 - cosine_similarity(&predicted, &example.next_state);
        }
        Ok(total / examples.len().max(1) as f32)
    }
}

fn validate_example(state_dim: usize, example: &LatentTransitionExample) -> Result<()> {
    if example.state.len() != state_dim
        || example.next_state.len() != state_dim
        || (!example.action.is_empty() && example.action.len() != state_dim)
    {
        bail!("transition example dimensions must match state_dim");
    }
    Ok(())
}

fn covariance_weight(
    examples: &[LatentTransitionExample],
    idx: usize,
    input_mean: f32,
    next_mean: f32,
    use_state: bool,
) -> f32 {
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for example in examples {
        let input = if use_state {
            example.state[idx]
        } else {
            example.action.get(idx).copied().unwrap_or_default()
        };
        numerator += (input - input_mean) * (example.next_state[idx] - next_mean);
        denominator += (input - input_mean).powi(2);
    }
    if denominator <= f32::EPSILON {
        if use_state { 1.0 } else { 0.0 }
    } else {
        (numerator / denominator).clamp(-2.0, 2.0)
    }
}

fn fit_auxiliary_heads(
    state_dim: usize,
    examples: &[LatentTransitionExample],
) -> Vec<AuxiliaryHead> {
    auxiliary_labels()
        .into_iter()
        .map(|label| fit_auxiliary_head(label, state_dim, examples))
        .collect()
}

fn fit_auxiliary_head(
    label: &'static str,
    state_dim: usize,
    examples: &[LatentTransitionExample],
) -> AuxiliaryHead {
    let positives = examples
        .iter()
        .filter(|example| label_value(&example.labels, label))
        .count() as f32;
    let prevalence = ((positives + 1.0) / (examples.len() as f32 + 2.0)).clamp(0.01, 0.99);
    let mut pos_state = vec![0.0; state_dim];
    let mut neg_state = vec![0.0; state_dim];
    let mut pos_action = vec![0.0; state_dim];
    let mut neg_action = vec![0.0; state_dim];
    let mut pos_count: f32 = 0.0;
    let mut neg_count: f32 = 0.0;
    for example in examples {
        let (state_target, action_target, count) = if label_value(&example.labels, label) {
            (&mut pos_state, &mut pos_action, &mut pos_count)
        } else {
            (&mut neg_state, &mut neg_action, &mut neg_count)
        };
        *count += 1.0;
        for idx in 0..state_dim {
            state_target[idx] += example.state[idx];
            action_target[idx] += example.action.get(idx).copied().unwrap_or_default();
        }
    }
    normalize_mean(&mut pos_state, pos_count);
    normalize_mean(&mut neg_state, neg_count);
    normalize_mean(&mut pos_action, pos_count);
    normalize_mean(&mut neg_action, neg_count);
    AuxiliaryHead {
        label: label.to_string(),
        bias: (prevalence / (1.0 - prevalence)).ln(),
        state_weights: pos_state
            .iter()
            .zip(&neg_state)
            .map(|(pos, neg)| (pos - neg).clamp(-1.0, 1.0) * 0.25)
            .collect(),
        action_weights: pos_action
            .iter()
            .zip(&neg_action)
            .map(|(pos, neg)| (pos - neg).clamp(-1.0, 1.0) * 0.25)
            .collect(),
    }
}

fn normalize_mean(values: &mut [f32], count: f32) {
    if count > 0.0 {
        for value in values {
            *value /= count;
        }
    }
}

fn auxiliary_labels() -> Vec<&'static str> {
    vec![
        "failure",
        "retry",
        "provider_incident",
        "verification_needed",
        "user_correction",
        "plan_drift",
        "high_cost",
        "slow_run",
    ]
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

fn dot_prefix(weights: &[f32], values: &[f32]) -> f32 {
    weights
        .iter()
        .zip(values)
        .map(|(weight, value)| weight * value)
        .sum()
}

fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value.clamp(-40.0, 40.0)).exp())
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm * right_norm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example(state: [f32; 2], action: [f32; 2], next_state: [f32; 2]) -> LatentTransitionExample {
        LatentTransitionExample {
            state: state.to_vec(),
            action: action.to_vec(),
            next_state: next_state.to_vec(),
            labels: WorldLabelSet::default(),
        }
    }

    #[test]
    fn cpu_transition_model_learns_mean_delta() {
        let examples = [
            example([0.0, 0.0], [0.0, 0.0], [1.0, 2.0]),
            example([1.0, 1.0], [0.0, 0.0], [2.0, 3.0]),
        ];

        let model = CpuLatentTransitionModel::fit(2, &examples).unwrap();
        let predicted = model.predict_next(&[2.0, 2.0], &[0.0, 0.0]).unwrap();

        assert_eq!(model.metadata.backend, BackendKind::Cpu);
        assert_eq!(model.mean_delta, vec![1.0, 2.0]);
        assert_eq!(model.metadata.parameter_count, 3 * 2 + 8 * (1 + 2 + 2));
        assert_eq!(predicted, vec![3.0, 4.0]);
    }

    #[test]
    fn cpu_transition_model_reports_low_training_error() {
        let examples = [
            example([0.0, 0.0], [0.0, 0.0], [1.0, 1.0]),
            example([2.0, 2.0], [0.0, 0.0], [3.0, 3.0]),
        ];

        let model = CpuLatentTransitionModel::fit(2, &examples).unwrap();

        assert!(model.mean_cosine_error(&examples).unwrap() < 0.01);
    }

    #[test]
    fn cpu_transition_model_predicts_auxiliary_heads() {
        let mut failed = example([1.0, 0.0], [0.0, 0.0], [1.0, 0.0]);
        failed.labels.failure = true;
        let ok = example([0.0, 1.0], [0.0, 0.0], [0.0, 1.0]);
        let model = CpuLatentTransitionModel::fit(2, &[failed, ok]).unwrap();

        let predictions = model.predict_auxiliary(&[1.0, 0.0], &[0.0, 0.0]).unwrap();

        assert!(predictions.iter().any(|head| head.label == "failure"));
    }
}
