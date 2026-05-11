use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::trace::{ColdStartStats, ColdStartStatus, ColdStartThresholds, evaluate_cold_start};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldPrediction {
    pub prediction_id: String,
    pub model_id: String,
    pub predicted_next_state_summary: String,
    pub evidence_refs: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl WorldPrediction {
    pub fn new(model_id: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            prediction_id: format!("world-prediction-{}", uuid::Uuid::new_v4()),
            model_id: model_id.into(),
            predicted_next_state_summary: summary.into(),
            evidence_refs: Vec::new(),
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldAdvisorUnavailableReason {
    ColdStart,
    Timeout,
    StoreUnavailable,
    CandidateOnly,
    TrainingInProgress,
    MissingEmbedding,
    MetalBackendFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldAdvisorUnavailable {
    pub reason: WorldAdvisorUnavailableReason,
    pub session_id: Option<String>,
    pub action_ref: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl WorldAdvisorUnavailable {
    pub fn new(reason: WorldAdvisorUnavailableReason) -> Self {
        Self {
            reason,
            session_id: None,
            action_ref: None,
            created_at: Utc::now(),
        }
    }

    pub fn with_context(
        mut self,
        session_id: impl Into<String>,
        action_ref: impl Into<String>,
    ) -> Self {
        self.session_id = Some(session_id.into());
        self.action_ref = Some(action_ref.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldAdvisorConfig {
    pub thresholds: ColdStartThresholds,
    pub active_model_id: Option<String>,
    pub training_in_progress: bool,
}

impl Default for WorldAdvisorConfig {
    fn default() -> Self {
        Self {
            thresholds: ColdStartThresholds::default(),
            active_model_id: None,
            training_in_progress: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldAdvisorContext {
    pub session_id: String,
    pub action_ref: String,
    pub action_summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldAdvisorDecision {
    pub prediction: Option<WorldPrediction>,
    pub unavailable: Option<WorldAdvisorUnavailable>,
}

#[derive(Debug, Clone)]
pub struct WorldAdvisor {
    config: WorldAdvisorConfig,
    stats: ColdStartStats,
}

impl WorldAdvisor {
    pub fn new(config: WorldAdvisorConfig, stats: ColdStartStats) -> Self {
        Self { config, stats }
    }

    pub fn predict_next(&self, context: &WorldAdvisorContext) -> Option<WorldPrediction> {
        self.evaluate(context).prediction
    }

    pub fn evaluate(&self, context: &WorldAdvisorContext) -> WorldAdvisorDecision {
        if self.config.training_in_progress {
            return unavailable(WorldAdvisorUnavailableReason::TrainingInProgress, context);
        }

        if !matches!(
            evaluate_cold_start(self.stats, self.config.thresholds),
            ColdStartStatus::Ready
        ) {
            return unavailable(WorldAdvisorUnavailableReason::ColdStart, context);
        }

        let Some(model_id) = &self.config.active_model_id else {
            return unavailable(WorldAdvisorUnavailableReason::CandidateOnly, context);
        };

        WorldAdvisorDecision {
            prediction: Some(WorldPrediction::new(
                model_id,
                format!(
                    "advisory next-state estimate for action '{}'",
                    context.action_summary
                ),
            )),
            unavailable: None,
        }
    }
}

fn unavailable(
    reason: WorldAdvisorUnavailableReason,
    context: &WorldAdvisorContext,
) -> WorldAdvisorDecision {
    WorldAdvisorDecision {
        prediction: None,
        unavailable: Some(
            WorldAdvisorUnavailable::new(reason)
                .with_context(context.session_id.clone(), context.action_ref.clone()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_stats() -> ColdStartStats {
        ColdStartStats {
            rows: 1_000,
            sessions: 50,
            observed_days: 7,
        }
    }

    fn context() -> WorldAdvisorContext {
        WorldAdvisorContext {
            session_id: "s1".into(),
            action_ref: "a1".into(),
            action_summary: "run verification".into(),
        }
    }

    #[test]
    fn advisor_returns_none_during_cold_start() {
        let advisor = WorldAdvisor::new(WorldAdvisorConfig::default(), ColdStartStats::default());
        let decision = advisor.evaluate(&context());

        assert!(decision.prediction.is_none());
        assert_eq!(
            decision.unavailable.unwrap().reason,
            WorldAdvisorUnavailableReason::ColdStart
        );
    }

    #[test]
    fn advisor_predicts_when_ready_and_active_model_exists() {
        let advisor = WorldAdvisor::new(
            WorldAdvisorConfig {
                active_model_id: Some("model-1".into()),
                ..WorldAdvisorConfig::default()
            },
            ready_stats(),
        );

        let prediction = advisor.predict_next(&context()).unwrap();

        assert_eq!(prediction.model_id, "model-1");
    }
}
