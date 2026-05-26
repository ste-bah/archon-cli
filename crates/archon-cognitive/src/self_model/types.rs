use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    DomainTrust,
    FailureCluster,
    CautionRule,
    Calibration,
    Capability,
    Correction,
}

impl FactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DomainTrust => "domain_trust",
            Self::FailureCluster => "failure_cluster",
            Self::CautionRule => "caution_rule",
            Self::Calibration => "calibration",
            Self::Capability => "capability",
            Self::Correction => "correction",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelfModelFact {
    pub id: String,
    pub domain: String,
    pub fact_kind: FactKind,
    pub statement: String,
    pub confidence: f32,
    pub evidence_count: u64,
    pub last_seen_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl SelfModelFact {
    pub fn new(
        domain: impl Into<String>,
        fact_kind: FactKind,
        statement: impl Into<String>,
        confidence: f32,
        evidence_count: u64,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            domain: domain.into(),
            fact_kind,
            statement: statement.into(),
            confidence: confidence.clamp(0.0, 1.0),
            evidence_count,
            last_seen_at: now,
            expires_at: None,
            created_at: now,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainTrust {
    pub domain: String,
    pub trust_score: f32,
    pub evidence_count: u64,
    pub last_correction_at: Option<DateTime<Utc>>,
    pub failure_cluster_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailureCluster {
    pub cluster_id: String,
    pub domain: String,
    pub pattern_summary: String,
    pub occurrence_count: u64,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub recommended_caution: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceCalibration {
    pub overall_calibration: f32,
    pub overconfidence_domains: Vec<String>,
    pub underconfidence_domains: Vec<String>,
}

impl Default for ConfidenceCalibration {
    fn default() -> Self {
        Self {
            overall_calibration: 0.5,
            overconfidence_domains: Vec::new(),
            underconfidence_domains: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelfModelProfile {
    pub domain_trust: Vec<DomainTrust>,
    pub active_failure_clusters: Vec<FailureCluster>,
    pub confidence_calibration: ConfidenceCalibration,
    pub caution_rules: Vec<String>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MemoryContext {
    pub fact_refs: Vec<String>,
    pub prior_correction_ids: Vec<String>,
    pub failure_pattern_labels: Vec<String>,
    pub context_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SelfModelBriefing {
    pub fact_count: usize,
    pub active_failure_clusters: usize,
    pub caution_rules: Vec<String>,
}
