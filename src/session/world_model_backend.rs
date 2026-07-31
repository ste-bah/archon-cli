//! Bridges the cognitive executive's [`PredictionBackend`] port to the world
//! model's advisor.
//!
//! This lives in the binary crate because it is the only one that depends on
//! both `archon-cognitive` and `archon-world-model`; `archon-core`, where the
//! advisory runs, cannot see the world model at all. The agent therefore holds
//! this behind a trait object and `session::cognitive_store` injects it.

use archon_cognitive::{
    Candidate, CognitiveError, ModelPrediction, PredictionBackend, PredictionDimensions,
    WorldModelState,
};
use archon_world_model::advisor::{WorldAdvisor, WorldAdvisorContext};
use archon_world_model::guardrail::GuardrailRiskScores;

/// Adapts [`WorldAdvisor`] to the cognitive executive's prediction port.
pub(super) struct WorldModelPredictionBackend {
    advisor: WorldAdvisor,
    session_id: String,
}

impl WorldModelPredictionBackend {
    pub(super) fn new(advisor: WorldAdvisor, session_id: impl Into<String>) -> Self {
        Self {
            advisor,
            session_id: session_id.into(),
        }
    }
}

impl PredictionBackend for WorldModelPredictionBackend {
    fn predict(
        &self,
        candidate: &Candidate,
        _state: &WorldModelState,
    ) -> Result<ModelPrediction, CognitiveError> {
        let context = WorldAdvisorContext {
            session_id: self.session_id.clone(),
            action_ref: candidate.id.clone(),
            action_summary: action_summary(candidate),
        };
        let prediction = self
            .advisor
            .predict_next(&context)
            .ok_or_else(|| CognitiveError::Store("world model returned no prediction".into()))?;
        // A prediction with no guardrail scores carries no risk signal, so it
        // must not be treated as "all risks are zero" — that would score the
        // candidate ABOVE its heuristic baseline. Refuse instead, and the
        // scorer falls back to the heuristic path.
        let scores = prediction.guardrail_scores.ok_or_else(|| {
            CognitiveError::Store("world model prediction carried no guardrail scores".into())
        })?;
        Ok(ModelPrediction {
            prediction_id: prediction.prediction_id,
            // `WorldModelScorer::score_with_model` clamps every dimension to
            // [0,1] before use, so no clamping is needed here.
            dimensions: dimensions_from(&scores, candidate),
        })
    }
}

/// Render a candidate as the free-text action summary the advisor embeds.
fn action_summary(candidate: &Candidate) -> String {
    let tool = candidate.tool_name.as_deref().unwrap_or("none");
    format!(
        "kind={:?} tool={tool} risk={:?} expected_evidence={} expected_output={}",
        candidate.action_kind,
        candidate.risk_class,
        candidate.expected_evidence,
        candidate.expected_user_output
    )
}

/// Map the world model's risk heads onto the executive's scoring dimensions.
///
/// The world model predicts RISK; it has no usefulness head. So
/// `expected_usefulness` carries the candidate's existing heuristic score
/// forward and the model's penalties are applied on top. Two consequences,
/// both deliberate:
///
/// * The model adjusts a baseline rather than replacing it — the discipline of
///   always comparing against your strongest simple baseline rather than
///   against zero. Defaulting this to 0.0 would floor every model-scored
///   candidate at ~0.20 and rank it below every heuristic-scored one.
/// * Penalties are subtractive, so the model can only ever LOWER a score. It
///   cannot promote a bad candidate on the strength of an unvalidated
///   prediction, which is the right failure mode until the eval gates have
///   been run.
fn dimensions_from(scores: &GuardrailRiskScores, candidate: &Candidate) -> PredictionDimensions {
    PredictionDimensions {
        predicted_risk: scores.predicted_failure.unwrap_or(0.0),
        retry_pressure: scores.predicted_retry.unwrap_or(0.0),
        verification_need: scores.predicted_verification_needed.unwrap_or(0.0),
        plan_drift_probability: scores.predicted_plan_drift.unwrap_or(0.0),
        // Provider incident rather than a second read of predicted_failure:
        // reusing that head would double-count the same signal at a combined
        // 0.35 weight in `composite_score`.
        likely_tool_failure: scores.predicted_provider_incident.unwrap_or(0.0),
        // No head produces this. 0.0 is the neutral value for a penalty term.
        // The k-NN counterfactual scoring in archon-world-model would be the
        // natural source if this is ever populated.
        similarity_to_prior_failures: 0.0,
        expected_usefulness: candidate.heuristic_score,
        expected_user_friction: scores.predicted_user_correction.unwrap_or(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_cognitive::{CandidateActionKind, RiskLevel, ScoreSource};

    fn candidate(heuristic: f32) -> Candidate {
        Candidate {
            id: "cand-1".into(),
            situation_id: "sit-1".into(),
            action_kind: CandidateActionKind::RunSafeShellProbe,
            tool_name: Some("bash".into()),
            expected_evidence: "command output".into(),
            expected_user_output: "result".into(),
            risk_class: RiskLevel::Medium,
            rollback_path: None,
            heuristic_score: heuristic,
            score_source: ScoreSource::Heuristic,
            created_at: chrono::Utc::now(),
        }
    }

    /// The model has no usefulness head, so it must carry the heuristic
    /// baseline forward. Zeroing it would floor every model-scored candidate
    /// at ~0.20 and rank it below every heuristic-scored one.
    #[test]
    fn expected_usefulness_carries_the_heuristic_baseline() {
        let dims = dimensions_from(&GuardrailRiskScores::default(), &candidate(0.75));
        assert_eq!(dims.expected_usefulness, 0.75);
    }

    /// A risk model must only ever be able to LOWER a score, never promote a
    /// candidate on the strength of an unvalidated prediction.
    #[test]
    fn risk_scores_can_only_reduce_the_composite() {
        let cand = candidate(0.9);
        let clean = dimensions_from(&GuardrailRiskScores::default(), &cand);
        let risky = dimensions_from(
            &GuardrailRiskScores {
                predicted_failure: Some(1.0),
                predicted_retry: Some(1.0),
                predicted_verification_needed: Some(1.0),
                predicted_plan_drift: Some(1.0),
                predicted_provider_incident: Some(1.0),
                predicted_user_correction: Some(1.0),
                ..GuardrailRiskScores::default()
            },
            &cand,
        );
        assert!(risky.composite_score() < clean.composite_score());
    }

    /// `predicted_failure` already drives `predicted_risk`; reading it a second
    /// time for `likely_tool_failure` would double-count one signal at a
    /// combined 0.35 weight.
    #[test]
    fn tool_failure_reads_provider_incident_not_predicted_failure() {
        let dims = dimensions_from(
            &GuardrailRiskScores {
                predicted_failure: Some(0.8),
                predicted_provider_incident: Some(0.2),
                ..GuardrailRiskScores::default()
            },
            &candidate(0.5),
        );
        assert_eq!(dims.predicted_risk, 0.8);
        assert_eq!(dims.likely_tool_failure, 0.2);
    }
}
