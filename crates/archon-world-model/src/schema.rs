use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub source: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl EvidenceRef {
    pub fn new(source: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            id: id.into(),
            path: None,
            hash: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingRef {
    pub embedding_id: String,
    pub provider: String,
    pub model: String,
    pub source_dimensions: usize,
    pub projection_dimensions: usize,
    pub source_hash: String,
    pub projection_version: String,
    pub redaction_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldTraceSource {
    ActivityEvent,
    PipelineBundle,
    ProviderRuntime,
    Plan,
    Conversation,
    AgentTranscript,
    AgentOutput,
    Retrospective,
    Memory,
    AgentEvolution,
    ReasoningQuality,
}

impl Default for WorldTraceSource {
    fn default() -> Self {
        Self::ActivityEvent
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldActionKind {
    AgentAttempt,
    ProviderCall,
    ToolCall,
    PlanUpdate,
    MemorySurface,
    Verification,
    Retry,
    Resume,
    Unknown,
}

impl Default for WorldActionKind {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldLabelSet {
    pub success: Option<bool>,
    pub failure: bool,
    pub retry: bool,
    pub provider_incident: bool,
    pub verification_needed: bool,
    pub user_correction: bool,
    pub plan_drift: bool,
    pub high_cost: bool,
    pub slow_run: bool,
}

impl Default for WorldLabelSet {
    fn default() -> Self {
        Self {
            success: None,
            failure: false,
            retry: false,
            provider_incident: false,
            verification_needed: false,
            user_correction: false,
            plan_drift: false,
            high_cost: false,
            slow_run: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScalarFeatures {
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub attempt_index: Option<u32>,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub quality_overall: Option<f64>,
    pub provider_cooldown_ms: Option<u64>,
}

impl Default for ScalarFeatures {
    fn default() -> Self {
        Self {
            cost_usd: None,
            duration_ms: None,
            attempt_index: None,
            tokens_in: None,
            tokens_out: None,
            quality_overall: None,
            provider_cooldown_ms: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldTraceRow {
    pub row_id: String,
    pub session_id: String,
    pub run_id: Option<String>,
    pub source: WorldTraceSource,
    pub action_kind: WorldActionKind,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub agent: Option<String>,
    pub state_embedding: Option<EmbeddingRef>,
    pub action_embedding: Option<EmbeddingRef>,
    pub next_state_embedding: Option<EmbeddingRef>,
    pub scalar_features: ScalarFeatures,
    pub labels: WorldLabelSet,
    pub evidence_refs: Vec<EvidenceRef>,
    pub redacted_excerpt: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Default for WorldTraceRow {
    fn default() -> Self {
        Self {
            row_id: String::new(),
            session_id: String::new(),
            run_id: None,
            source: WorldTraceSource::default(),
            action_kind: WorldActionKind::default(),
            provider: None,
            model: None,
            agent: None,
            state_embedding: None,
            action_embedding: None,
            next_state_embedding: None,
            scalar_features: ScalarFeatures::default(),
            labels: WorldLabelSet::default(),
            evidence_refs: Vec::new(),
            redacted_excerpt: None,
            created_at: Utc::now(),
        }
    }
}

impl WorldTraceRow {
    pub fn new(session_id: impl Into<String>, action_kind: WorldActionKind) -> Self {
        Self {
            row_id: format!("world-row-{}", uuid::Uuid::new_v4()),
            session_id: session_id.into(),
            action_kind,
            created_at: Utc::now(),
            ..Self::default()
        }
    }

    pub fn with_evidence(mut self, evidence: EvidenceRef) -> Self {
        self.evidence_refs.push(evidence);
        self
    }

    pub fn with_row_id(mut self, row_id: impl Into<String>) -> Self {
        self.row_id = row_id.into();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_trace_row_sets_stable_prefix() {
        let row = WorldTraceRow::new("session-1", WorldActionKind::AgentAttempt);
        assert!(row.row_id.starts_with("world-row-"));
        assert_eq!(row.session_id, "session-1");
        assert_eq!(row.action_kind, WorldActionKind::AgentAttempt);
    }
}
