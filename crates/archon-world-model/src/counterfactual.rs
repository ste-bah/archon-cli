//! Similarity-based counterfactual scoring.
//!
//! This is intentionally k-NN over historical action embeddings, not causal
//! inference. Calibration is reported separately for observed and hypothetical
//! action paths.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterfactualExample {
    pub action_id: String,
    pub action_embedding: Vec<f32>,
    pub observed_success: f32,
    pub observed_risk: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NeighborScore {
    pub action_id: String,
    pub similarity: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuxiliaryRiskPrediction {
    pub failure: f32,
    pub retry: f32,
    pub provider_incident: f32,
    pub verification_needed: f32,
    pub plan_drift: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterfactualScore {
    pub candidate_id: String,
    pub estimated_success: f32,
    pub estimated_risk: f32,
    pub auxiliary: AuxiliaryRiskPrediction,
    pub neighbors: Vec<NeighborScore>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterfactualCalibrationReport {
    pub observed_action_ece: f32,
    pub counterfactual_ece: f32,
    pub note: String,
}

#[derive(Debug, Clone)]
pub struct KnnCounterfactualAdvisor {
    examples: Vec<CounterfactualExample>,
    k: usize,
}

impl KnnCounterfactualAdvisor {
    pub fn new(examples: Vec<CounterfactualExample>, k: usize) -> Result<Self> {
        if k == 0 {
            bail!("k must be greater than zero");
        }
        Ok(Self { examples, k })
    }

    pub fn score(&self, candidate_id: &str, embedding: &[f32]) -> Result<CounterfactualScore> {
        if self.examples.is_empty() {
            bail!("counterfactual scoring requires historical examples");
        }

        let mut ranked = self
            .examples
            .iter()
            .map(|example| {
                Ok((
                    example,
                    cosine_similarity(embedding, &example.action_embedding)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        ranked.sort_by(|left, right| right.1.total_cmp(&left.1));

        let selected = ranked.into_iter().take(self.k).collect::<Vec<_>>();
        let weight_sum = selected
            .iter()
            .map(|(_, similarity)| similarity.max(0.0))
            .sum::<f32>()
            .max(0.000_001);
        let estimated_success =
            weighted_average(&selected, weight_sum, |example| example.observed_success);
        let estimated_risk =
            weighted_average(&selected, weight_sum, |example| example.observed_risk);

        Ok(CounterfactualScore {
            candidate_id: candidate_id.to_string(),
            estimated_success,
            estimated_risk,
            auxiliary: auxiliary_from_risk(estimated_risk),
            neighbors: selected
                .into_iter()
                .map(|(example, similarity)| NeighborScore {
                    action_id: example.action_id.clone(),
                    similarity,
                })
                .collect(),
        })
    }
}

pub fn calibration_report(
    observed_ece: f32,
    counterfactual_ece: f32,
) -> CounterfactualCalibrationReport {
    CounterfactualCalibrationReport {
        observed_action_ece: observed_ece,
        counterfactual_ece,
        note: "counterfactual scores are similarity-based, not causal".into(),
    }
}

fn weighted_average(
    selected: &[(&CounterfactualExample, f32)],
    weight_sum: f32,
    value: impl Fn(&CounterfactualExample) -> f32,
) -> f32 {
    selected
        .iter()
        .map(|(example, similarity)| similarity.max(0.0) * value(example))
        .sum::<f32>()
        / weight_sum
}

fn auxiliary_from_risk(risk: f32) -> AuxiliaryRiskPrediction {
    AuxiliaryRiskPrediction {
        failure: risk,
        retry: (risk * 0.75).min(1.0),
        provider_incident: (risk * 0.5).min(1.0),
        verification_needed: (risk * 0.8).min(1.0),
        plan_drift: (risk * 0.35).min(1.0),
    }
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> Result<f32> {
    if left.len() != right.len() {
        bail!("embedding dimensions must match");
    }
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        Ok(0.0)
    } else {
        Ok(dot / (left_norm * right_norm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example(id: &str, embedding: &[f32], success: f32, risk: f32) -> CounterfactualExample {
        CounterfactualExample {
            action_id: id.into(),
            action_embedding: embedding.to_vec(),
            observed_success: success,
            observed_risk: risk,
        }
    }

    #[test]
    fn knn_scores_candidate_from_nearest_actions() {
        let advisor = KnnCounterfactualAdvisor::new(
            vec![
                example("good", &[1.0, 0.0], 0.9, 0.1),
                example("bad", &[0.0, 1.0], 0.1, 0.9),
            ],
            1,
        )
        .unwrap();

        let score = advisor.score("candidate", &[1.0, 0.0]).unwrap();

        assert_eq!(score.neighbors[0].action_id, "good");
        assert!(score.estimated_success > score.estimated_risk);
        assert!(score.auxiliary.failure < 0.2);
    }

    #[test]
    fn calibration_report_states_counterfactual_limit() {
        let report = calibration_report(0.05, 0.12);

        assert_eq!(report.observed_action_ece, 0.05);
        assert!(report.note.contains("not causal"));
    }
}
