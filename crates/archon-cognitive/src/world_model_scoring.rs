use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    Candidate, CandidateActionKind, CandidateScore, CognitiveError, RiskLevel, ScoreSource,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    LatentTransition,
    JepaTransition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorldModelState {
    pub active_model_id: Option<String>,
    pub active_model_kind: Option<ModelKind>,
    pub jepa_promoted: bool,
    pub shadow_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PredictionDimensions {
    pub predicted_risk: f32,
    pub retry_pressure: f32,
    pub verification_need: f32,
    pub plan_drift_probability: f32,
    pub likely_tool_failure: f32,
    pub similarity_to_prior_failures: f32,
    pub expected_usefulness: f32,
    pub expected_user_friction: f32,
}

impl PredictionDimensions {
    pub fn composite_score(&self) -> f32 {
        let penalties = self.predicted_risk * 0.20
            + self.retry_pressure * 0.10
            + self.verification_need * 0.10
            + self.plan_drift_probability * 0.15
            + self.likely_tool_failure * 0.15
            + self.similarity_to_prior_failures * 0.10
            + self.expected_user_friction * 0.10;
        (0.20 + self.expected_usefulness * 0.80 - penalties).clamp(0.0, 1.0)
    }

    fn clamp(mut self) -> Self {
        self.predicted_risk = clamp01(self.predicted_risk);
        self.retry_pressure = clamp01(self.retry_pressure);
        self.verification_need = clamp01(self.verification_need);
        self.plan_drift_probability = clamp01(self.plan_drift_probability);
        self.likely_tool_failure = clamp01(self.likely_tool_failure);
        self.similarity_to_prior_failures = clamp01(self.similarity_to_prior_failures);
        self.expected_usefulness = clamp01(self.expected_usefulness);
        self.expected_user_friction = clamp01(self.expected_user_friction);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPrediction {
    pub prediction_id: String,
    pub dimensions: PredictionDimensions,
}

pub trait PredictionBackend {
    fn predict(
        &self,
        candidate: &Candidate,
        state: &WorldModelState,
    ) -> Result<ModelPrediction, CognitiveError>;
}

#[derive(Debug, Clone, Default)]
pub struct NoopPredictionBackend;

impl PredictionBackend for NoopPredictionBackend {
    fn predict(
        &self,
        _candidate: &Candidate,
        _state: &WorldModelState,
    ) -> Result<ModelPrediction, CognitiveError> {
        Err(CognitiveError::Store(
            "prediction backend unavailable".into(),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct WorldModelScorer<B> {
    backend: Option<B>,
    pub world_model_enabled: bool,
    pub jepa_enabled: bool,
    pub model_timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoredCandidates {
    pub candidates: Vec<Candidate>,
    pub scores: Vec<CandidateScore>,
    pub prediction_ids: Vec<Option<String>>,
    pub prediction_available: bool,
    pub model_id: Option<String>,
    pub model_kind: Option<ModelKind>,
    pub scored_at: DateTime<Utc>,
}

impl WorldModelScorer<NoopPredictionBackend> {
    pub fn heuristic_only() -> Self {
        Self {
            backend: Some(NoopPredictionBackend),
            world_model_enabled: false,
            jepa_enabled: false,
            model_timeout_ms: 200,
        }
    }
}

impl<B: PredictionBackend> WorldModelScorer<B> {
    pub fn new(backend: B, world_model_enabled: bool, jepa_enabled: bool) -> Self {
        Self {
            backend: Some(backend),
            world_model_enabled,
            jepa_enabled,
            model_timeout_ms: 200,
        }
    }

    pub fn score(&self, candidates: &[Candidate], state: &WorldModelState) -> ScoredCandidates {
        let model_source = self.model_score_source(state);
        let mut prediction_available = false;
        let mut scored_candidates = Vec::with_capacity(candidates.len());
        let mut scores = Vec::with_capacity(candidates.len());
        let mut prediction_ids = Vec::with_capacity(candidates.len());

        for candidate in candidates {
            let scored = match model_source {
                Some(source) => self.score_with_model(candidate, state, source),
                None => None,
            };
            let (mut candidate, score, prediction_id) =
                scored.unwrap_or_else(|| fallback_candidate(candidate));
            prediction_available |= prediction_id.is_some();
            scores.push(score);
            prediction_ids.push(prediction_id);
            candidate.heuristic_score = candidate.heuristic_score.clamp(0.0, 1.0);
            scored_candidates.push(candidate);
        }

        ScoredCandidates {
            candidates: scored_candidates,
            scores,
            prediction_ids,
            prediction_available,
            model_id: state.active_model_id.clone(),
            model_kind: state.active_model_kind,
            scored_at: Utc::now(),
        }
    }

    fn score_with_model(
        &self,
        candidate: &Candidate,
        state: &WorldModelState,
        source: ScoreSource,
    ) -> Option<(Candidate, CandidateScore, Option<String>)> {
        let prediction = self.backend.as_ref()?.predict(candidate, state).ok()?;
        let dimensions = prediction.dimensions.clamp();
        let mut scored = candidate.clone();
        scored.heuristic_score = dimensions.composite_score();
        scored.score_source = source;
        let score = CandidateScore {
            candidate_id: candidate.id.clone(),
            score: scored.heuristic_score,
            score_source: source,
            rationale: format!("model prediction {}", prediction.prediction_id),
        };
        Some((scored, score, Some(prediction.prediction_id)))
    }

    fn model_score_source(&self, state: &WorldModelState) -> Option<ScoreSource> {
        if state.active_model_id.is_none() || state.shadow_only {
            return None;
        }
        match state.active_model_kind {
            Some(ModelKind::LatentTransition) if self.world_model_enabled => {
                Some(ScoreSource::LatentTransition)
            }
            Some(ModelKind::JepaTransition) if self.jepa_enabled && state.jepa_promoted => {
                Some(ScoreSource::JepaTransition)
            }
            _ => None,
        }
    }
}

fn fallback_candidate(candidate: &Candidate) -> (Candidate, CandidateScore, Option<String>) {
    let dimensions = heuristic_dimensions(candidate);
    let mut scored = candidate.clone();
    scored.heuristic_score = dimensions.composite_score();
    scored.score_source = ScoreSource::PredictionUnavailable;
    let score = CandidateScore {
        candidate_id: candidate.id.clone(),
        score: scored.heuristic_score,
        score_source: ScoreSource::PredictionUnavailable,
        rationale: "prediction unavailable; heuristic fallback".into(),
    };
    (scored, score, None)
}

fn heuristic_dimensions(candidate: &Candidate) -> PredictionDimensions {
    let risk = risk_value(candidate.risk_class);
    let friction = friction_value(candidate.action_kind);
    PredictionDimensions {
        predicted_risk: risk,
        retry_pressure: if candidate.action_kind == CandidateActionKind::RunTests {
            0.35
        } else {
            risk * 0.6
        },
        verification_need: verification_need(candidate.action_kind),
        plan_drift_probability: if candidate.action_kind == CandidateActionKind::AskClarification {
            0.1
        } else {
            risk * 0.7
        },
        likely_tool_failure: tool_failure(candidate.action_kind),
        similarity_to_prior_failures: 0.0,
        expected_usefulness: usefulness(candidate.action_kind),
        expected_user_friction: friction,
    }
    .clamp()
}

fn risk_value(risk: RiskLevel) -> f32 {
    match risk {
        RiskLevel::Low => 0.1,
        RiskLevel::Medium => 0.45,
        RiskLevel::High => 0.75,
        RiskLevel::Critical => 1.0,
    }
}

fn verification_need(kind: CandidateActionKind) -> f32 {
    match kind {
        CandidateActionKind::RunTests
        | CandidateActionKind::RunSafeShellProbe
        | CandidateActionKind::RunLearningTick => 0.7,
        CandidateActionKind::InspectFiles | CandidateActionKind::SearchDocs => 0.45,
        _ => 0.2,
    }
}

fn tool_failure(kind: CandidateActionKind) -> f32 {
    match kind {
        CandidateActionKind::RunSafeShellProbe | CandidateActionKind::RunTests => 0.35,
        CandidateActionKind::SearchDocs | CandidateActionKind::RecallMemory => 0.2,
        _ => 0.05,
    }
}

fn usefulness(kind: CandidateActionKind) -> f32 {
    match kind {
        CandidateActionKind::InspectFiles
        | CandidateActionKind::SearchDocs
        | CandidateActionKind::RunTests => 0.85,
        CandidateActionKind::AskClarification => 0.75,
        CandidateActionKind::AnswerDirectly => 0.65,
        CandidateActionKind::DeferOrDecline => 0.35,
        _ => 0.6,
    }
}

fn friction_value(kind: CandidateActionKind) -> f32 {
    match kind {
        CandidateActionKind::AnswerDirectly => 0.05,
        CandidateActionKind::AskClarification => 0.1,
        CandidateActionKind::RecallMemory => 0.2,
        CandidateActionKind::SearchDocs => 0.3,
        CandidateActionKind::InspectFiles => 0.35,
        CandidateActionKind::RunSafeShellProbe => 0.5,
        CandidateActionKind::RunTests => 0.6,
        CandidateActionKind::RunLearningTick => 0.65,
        CandidateActionKind::CreateGovernedProposal => 0.75,
        CandidateActionKind::DeferOrDecline => 0.8,
    }
}

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}
