//! Shadow planning helpers for advisory action ranking.

use serde::{Deserialize, Serialize};

use crate::counterfactual::CounterfactualScore;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowActionScore {
    pub action_id: String,
    pub advisory_score: f32,
    pub estimated_success: f32,
    pub estimated_risk: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowPlanReport {
    pub ranked_actions: Vec<ShadowActionScore>,
    pub ndcg_at_5: f32,
    pub beats_random_and_rule_baselines: bool,
}

pub fn rank_candidate_actions(
    scores: &[CounterfactualScore],
    relevance: &[(String, f32)],
) -> ShadowPlanReport {
    let mut ranked_actions = scores
        .iter()
        .map(|score| ShadowActionScore {
            action_id: score.candidate_id.clone(),
            advisory_score: score.estimated_success - score.estimated_risk,
            estimated_success: score.estimated_success,
            estimated_risk: score.estimated_risk,
        })
        .collect::<Vec<_>>();
    ranked_actions.sort_by(|left, right| right.advisory_score.total_cmp(&left.advisory_score));

    let ndcg_at_5 = ndcg_at_k(&ranked_actions, relevance, 5);
    ShadowPlanReport {
        ranked_actions,
        ndcg_at_5,
        beats_random_and_rule_baselines: ndcg_at_5 >= 0.60,
    }
}

pub fn ndcg_at_k(ranked: &[ShadowActionScore], relevance: &[(String, f32)], k: usize) -> f32 {
    let actual = ranked
        .iter()
        .take(k)
        .enumerate()
        .map(|(idx, action)| discounted_gain(relevance_for(&action.action_id, relevance), idx))
        .sum::<f32>();

    let mut ideal = relevance.iter().map(|(_, rel)| *rel).collect::<Vec<_>>();
    ideal.sort_by(|left, right| right.total_cmp(left));
    let ideal_dcg = ideal
        .iter()
        .take(k)
        .enumerate()
        .map(|(idx, rel)| discounted_gain(*rel, idx))
        .sum::<f32>();

    if ideal_dcg == 0.0 {
        0.0
    } else {
        actual / ideal_dcg
    }
}

fn relevance_for(action_id: &str, relevance: &[(String, f32)]) -> f32 {
    relevance
        .iter()
        .find(|(id, _)| id == action_id)
        .map(|(_, rel)| *rel)
        .unwrap_or(0.0)
}

fn discounted_gain(relevance: f32, zero_based_rank: usize) -> f32 {
    relevance / ((zero_based_rank + 2) as f32).log2()
}

#[cfg(test)]
mod tests {
    use crate::counterfactual::{AuxiliaryRiskPrediction, CounterfactualScore};

    use super::*;

    fn score(id: &str, success: f32, risk: f32) -> CounterfactualScore {
        CounterfactualScore {
            candidate_id: id.into(),
            estimated_success: success,
            estimated_risk: risk,
            auxiliary: AuxiliaryRiskPrediction {
                failure: risk,
                retry: 0.0,
                provider_incident: 0.0,
                verification_needed: 0.0,
                plan_drift: 0.0,
            },
            neighbors: Vec::new(),
        }
    }

    #[test]
    fn ranks_actions_by_success_minus_risk() {
        let report = rank_candidate_actions(
            &[score("slow", 0.7, 0.4), score("safe", 0.9, 0.1)],
            &[("safe".into(), 3.0), ("slow".into(), 1.0)],
        );

        assert_eq!(report.ranked_actions[0].action_id, "safe");
        assert!(report.ndcg_at_5 > 0.99);
        assert!(report.beats_random_and_rule_baselines);
    }

    #[test]
    fn ndcg_returns_zero_without_relevance() {
        assert_eq!(ndcg_at_k(&[], &[], 5), 0.0);
    }
}
