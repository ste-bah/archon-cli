use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::run::StageStatus;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum QualityGateDecision {
    Accept,
    Retry,
    Fail,
    ForcedAccept(ForcedAcceptAudit),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForcedAcceptAudit {
    pub forced_by: String,
    pub rationale: String,
    pub source: String,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QualityGate {
    pub threshold: f64,
    pub max_attempts: u32,
}

impl Default for QualityGate {
    fn default() -> Self {
        Self {
            threshold: 0.50,
            max_attempts: 3,
        }
    }
}

impl QualityGate {
    pub fn decide(&self, score: f64, attempts: u32) -> QualityGateDecision {
        if score >= self.threshold {
            QualityGateDecision::Accept
        } else if attempts < self.max_attempts {
            QualityGateDecision::Retry
        } else {
            QualityGateDecision::Fail
        }
    }

    pub fn force_accept(
        &self,
        forced_by: impl Into<String>,
        rationale: impl Into<String>,
        source: impl Into<String>,
    ) -> QualityGateDecision {
        QualityGateDecision::ForcedAccept(ForcedAcceptAudit {
            forced_by: forced_by.into(),
            rationale: rationale.into(),
            source: source.into(),
            ts: Utc::now(),
        })
    }
}

pub fn status_for_decision(decision: &QualityGateDecision) -> StageStatus {
    match decision {
        QualityGateDecision::Accept => StageStatus::Accepted,
        QualityGateDecision::Retry => StageStatus::Pending,
        QualityGateDecision::Fail => StageStatus::Failed,
        QualityGateDecision::ForcedAccept(_) => StageStatus::ForcedAccepted,
    }
}
