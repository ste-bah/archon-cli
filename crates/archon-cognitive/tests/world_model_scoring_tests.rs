use archon_cognitive::{
    Candidate, CandidateActionKind, CognitiveError, ModelKind, ModelPrediction, PredictionBackend,
    PredictionDimensions, RiskLevel, ScoreSource, WorldModelScorer, WorldModelState,
};
use chrono::Utc;

#[derive(Clone)]
struct MockBackend;

impl PredictionBackend for MockBackend {
    fn predict(
        &self,
        candidate: &Candidate,
        _state: &WorldModelState,
    ) -> Result<ModelPrediction, CognitiveError> {
        Ok(ModelPrediction {
            prediction_id: format!("prediction-{}", candidate.id),
            dimensions: PredictionDimensions {
                predicted_risk: 0.1,
                retry_pressure: 0.1,
                verification_need: 0.2,
                plan_drift_probability: 0.1,
                likely_tool_failure: 0.1,
                similarity_to_prior_failures: 0.0,
                expected_usefulness: 0.9,
                expected_user_friction: 0.1,
            },
        })
    }
}

fn candidate(kind: CandidateActionKind, risk: RiskLevel) -> Candidate {
    Candidate {
        id: format!("candidate-{}", kind.as_str()),
        situation_id: "situation-1".into(),
        action_kind: kind,
        tool_name: None,
        expected_evidence: "file contents".into(),
        expected_user_output: "summary".into(),
        risk_class: risk,
        rollback_path: None,
        heuristic_score: 0.4,
        score_source: ScoreSource::Heuristic,
        created_at: Utc::now(),
    }
}

#[test]
fn no_active_model_falls_back_to_prediction_unavailable() {
    let scorer = WorldModelScorer::heuristic_only();
    let result = scorer.score(
        &[candidate(CandidateActionKind::InspectFiles, RiskLevel::Low)],
        &WorldModelState::default(),
    );

    assert!(!result.prediction_available);
    assert_eq!(result.prediction_ids, [None]);
    assert_eq!(
        result.scores[0].score_source,
        ScoreSource::PredictionUnavailable
    );
    assert_eq!(
        result.candidates[0].score_source,
        ScoreSource::PredictionUnavailable
    );
}

#[test]
fn active_latent_model_scores_and_links_prediction_id() {
    let scorer = WorldModelScorer::new(MockBackend, true, false);
    let state = WorldModelState {
        active_model_id: Some("latent-1".into()),
        active_model_kind: Some(ModelKind::LatentTransition),
        ..WorldModelState::default()
    };
    let result = scorer.score(
        &[candidate(CandidateActionKind::InspectFiles, RiskLevel::Low)],
        &state,
    );

    assert!(result.prediction_available);
    assert_eq!(result.model_id.as_deref(), Some("latent-1"));
    assert_eq!(
        result.prediction_ids[0].as_deref(),
        Some("prediction-candidate-inspect_files")
    );
    assert_eq!(result.scores[0].score_source, ScoreSource::LatentTransition);
}

#[test]
fn unpromoted_jepa_uses_heuristic_decision_path() {
    let scorer = WorldModelScorer::new(MockBackend, false, true);
    let state = WorldModelState {
        active_model_id: Some("jepa-candidate".into()),
        active_model_kind: Some(ModelKind::JepaTransition),
        jepa_promoted: false,
        ..WorldModelState::default()
    };
    let result = scorer.score(
        &[candidate(CandidateActionKind::RunTests, RiskLevel::Medium)],
        &state,
    );

    assert!(!result.prediction_available);
    assert_eq!(
        result.scores[0].score_source,
        ScoreSource::PredictionUnavailable
    );
}

#[test]
fn prediction_dimensions_are_clamped_before_scoring() {
    let dimensions = PredictionDimensions {
        predicted_risk: -1.0,
        retry_pressure: 2.0,
        verification_need: 0.0,
        plan_drift_probability: 0.0,
        likely_tool_failure: 0.0,
        similarity_to_prior_failures: 0.0,
        expected_usefulness: 2.0,
        expected_user_friction: -2.0,
    };

    assert!((0.0..=1.0).contains(&dimensions.composite_score()));
}
