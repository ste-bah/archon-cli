use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowEvaluationVerdict {
    Promote,
    NeedsReview,
    Reject,
    Inconclusive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentShadowEvaluation {
    pub evaluation_id: String,
    pub proposal_id: String,
    pub agent_type: String,
    pub candidate_version_id: Option<String>,
    pub baseline_version_id: Option<String>,
    pub task_set_id: Option<String>,
    pub baseline_score: f64,
    pub candidate_score: f64,
    pub regression_count: u32,
    pub improvement_count: u32,
    pub verdict: ShadowEvaluationVerdict,
    pub evidence_json: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl AgentShadowEvaluation {
    pub fn new(
        proposal_id: impl Into<String>,
        agent_type: impl Into<String>,
        baseline_score: f64,
        candidate_score: f64,
        regression_count: u32,
        improvement_count: u32,
    ) -> Self {
        Self {
            evaluation_id: agent_shadow_evaluation_id(),
            proposal_id: proposal_id.into(),
            agent_type: agent_type.into(),
            candidate_version_id: None,
            baseline_version_id: None,
            task_set_id: None,
            baseline_score: clamp_unit(baseline_score),
            candidate_score: clamp_unit(candidate_score),
            regression_count,
            improvement_count,
            verdict: verdict_from_scores(
                baseline_score,
                candidate_score,
                regression_count,
                improvement_count,
            ),
            evidence_json: serde_json::json!({}),
            created_at: Utc::now(),
        }
    }

    pub fn with_candidate_version(mut self, version_id: impl Into<String>) -> Self {
        self.candidate_version_id = Some(version_id.into());
        self
    }

    pub fn with_baseline_version(mut self, version_id: impl Into<String>) -> Self {
        self.baseline_version_id = Some(version_id.into());
        self
    }

    pub fn with_task_set(mut self, task_set_id: impl Into<String>) -> Self {
        self.task_set_id = Some(task_set_id.into());
        self
    }

    pub fn with_evidence(mut self, evidence_json: serde_json::Value) -> Self {
        self.evidence_json = evidence_json;
        self
    }
}

pub fn agent_shadow_evaluation_id() -> String {
    format!("agent-shadow-{}", uuid::Uuid::new_v4())
}

fn verdict_from_scores(
    baseline_score: f64,
    candidate_score: f64,
    regression_count: u32,
    improvement_count: u32,
) -> ShadowEvaluationVerdict {
    if improvement_count == 0 && regression_count == 0 {
        return ShadowEvaluationVerdict::Inconclusive;
    }

    if candidate_score < baseline_score && regression_count > 0 {
        return ShadowEvaluationVerdict::Reject;
    }

    if regression_count > 0 {
        return ShadowEvaluationVerdict::NeedsReview;
    }

    if candidate_score >= baseline_score && improvement_count > 0 {
        return ShadowEvaluationVerdict::Promote;
    }

    ShadowEvaluationVerdict::Inconclusive
}

fn clamp_unit(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_evaluation_serializes_core_prd_fields() {
        let evaluation = AgentShadowEvaluation::new("prop-1", "reviewer", 0.72, 0.81, 0, 3)
            .with_baseline_version("agent-profile-1")
            .with_candidate_version("agent-profile-2")
            .with_task_set("shadow-set-1")
            .with_evidence(serde_json::json!({"tasks": 5}));

        let value = serde_json::to_value(&evaluation).unwrap();

        assert!(evaluation.evaluation_id.starts_with("agent-shadow-"));
        assert_eq!(value["proposal_id"], "prop-1");
        assert_eq!(value["agent_type"], "reviewer");
        assert_eq!(value["verdict"], "promote");
        assert_eq!(evaluation.evidence_json["tasks"], 5);
    }

    #[test]
    fn regressions_force_review_or_rejection() {
        let lower_with_regression = AgentShadowEvaluation::new("prop-1", "coder", 0.8, 0.7, 1, 2);
        let higher_with_regression = AgentShadowEvaluation::new("prop-2", "coder", 0.8, 0.9, 1, 4);

        assert_eq!(
            lower_with_regression.verdict,
            ShadowEvaluationVerdict::Reject
        );
        assert_eq!(
            higher_with_regression.verdict,
            ShadowEvaluationVerdict::NeedsReview
        );
    }

    #[test]
    fn no_shadow_signal_is_inconclusive() {
        let evaluation = AgentShadowEvaluation::new("prop-1", "planner", 0.5, 0.5, 0, 0);

        assert_eq!(evaluation.verdict, ShadowEvaluationVerdict::Inconclusive);
    }

    #[test]
    fn scores_are_clamped_for_storage() {
        let evaluation = AgentShadowEvaluation::new("prop-1", "planner", -1.0, 2.0, 0, 1);

        assert_eq!(evaluation.baseline_score, 0.0);
        assert_eq!(evaluation.candidate_score, 1.0);
        assert_eq!(evaluation.verdict, ShadowEvaluationVerdict::Promote);
    }
}
