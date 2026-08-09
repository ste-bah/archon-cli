//! Immutable evaluation windows and the cohort segmentation they carry.
//!
//! A window is a frozen declaration of *what population was measured*.
//! Redefining one would silently rewrite history, so the store rejects a
//! redefinition rather than upserting it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::CognitiveError;

/// Placeholder used by the pooled, unsegmented cohort.
pub const COHORT_WILDCARD: &str = "*";

/// Segmentation identity. The roadmap forbids aggregate-only trends, so every
/// derived metric is reported per task class and model/policy version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MetricCohort {
    pub task_class: String,
    pub model_id: String,
    pub policy_version: String,
}

impl MetricCohort {
    pub fn new(
        task_class: impl Into<String>,
        model_id: impl Into<String>,
        policy_version: impl Into<String>,
    ) -> Self {
        Self {
            task_class: task_class.into(),
            model_id: model_id.into(),
            policy_version: policy_version.into(),
        }
    }

    /// The pooled cohort reported alongside the segments, never instead of them.
    pub fn pooled() -> Self {
        Self::new(COHORT_WILDCARD, COHORT_WILDCARD, COHORT_WILDCARD)
    }

    pub fn is_pooled(&self) -> bool {
        self.task_class == COHORT_WILDCARD
            && self.model_id == COHORT_WILDCARD
            && self.policy_version == COHORT_WILDCARD
    }

    pub fn segmentation_key(&self) -> String {
        format!(
            "{}/{}/{}",
            self.task_class, self.model_id, self.policy_version
        )
    }
}

/// Which side of a comparison a window represents.
///
/// Baseline/canary comparison itself is deliberately not implemented yet; the
/// role is recorded now so windows written today remain usable when it lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CohortRole {
    Baseline,
    Canary,
    Observational,
}

impl CohortRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Canary => "canary",
            Self::Observational => "observational",
        }
    }
}

impl std::str::FromStr for CohortRole {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "baseline" => Self::Baseline,
            "canary" => Self::Canary,
            "observational" => Self::Observational,
            _ => return Err(()),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationWindow {
    pub evaluation_window_id: String,
    pub label: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    /// Version of the eligible-population query, so a later query change
    /// cannot be mistaken for a behaviour change in the measured system.
    pub population_query_version: i64,
    pub segmentation_keys: Vec<String>,
    pub cohort_role: CohortRole,
    pub cohort_identity: String,
    pub metric_definition_version: i64,
    pub created_at: DateTime<Utc>,
}

impl EvaluationWindow {
    pub fn new(
        evaluation_window_id: impl Into<String>,
        started_at: DateTime<Utc>,
        ended_at: DateTime<Utc>,
    ) -> Self {
        let evaluation_window_id = evaluation_window_id.into();
        Self {
            label: evaluation_window_id.clone(),
            evaluation_window_id,
            started_at,
            ended_at,
            population_query_version: 1,
            segmentation_keys: default_segmentation_keys(),
            cohort_role: CohortRole::Observational,
            cohort_identity: String::new(),
            metric_definition_version: crate::metrics::METRIC_DEFINITION_VERSION,
            created_at: started_at,
        }
    }

    pub fn with_role(mut self, role: CohortRole, cohort_identity: impl Into<String>) -> Self {
        self.cohort_role = role;
        self.cohort_identity = cohort_identity.into();
        self
    }

    /// Half-open `[started_at, ended_at)` so adjacent windows cannot
    /// double-count an event that lands exactly on the boundary.
    pub fn contains(&self, at: DateTime<Utc>) -> bool {
        at >= self.started_at && at < self.ended_at
    }

    pub fn validate(&self) -> Result<(), CognitiveError> {
        if self.evaluation_window_id.trim().is_empty() {
            return Err(invalid("evaluation_window_id must not be empty"));
        }
        if self.ended_at <= self.started_at {
            return Err(invalid("ended_at must be after started_at"));
        }
        if self.segmentation_keys.is_empty() {
            return Err(invalid("segmentation_keys must not be empty"));
        }
        if self.population_query_version <= 0 {
            return Err(invalid("population_query_version must be positive"));
        }
        if self.metric_definition_version <= 0 {
            return Err(invalid("metric_definition_version must be positive"));
        }
        Ok(())
    }
}

pub fn default_segmentation_keys() -> Vec<String> {
    vec![
        "task_class".to_string(),
        "model_id".to_string(),
        "policy_version".to_string(),
    ]
}

fn invalid(message: &str) -> CognitiveError {
    CognitiveError::Metric(format!("invalid evaluation window: {message}"))
}
