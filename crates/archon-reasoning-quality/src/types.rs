use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEventKind {
    ClaimEmitted,
    UncertaintyDisclosed,
    UnsupportedClaim,
    ClaimBeforeSourceRead,
    SourceVerifiedClaim,
    ClaimContradictedBySource,
    ClaimCorrectedByUser,
    VerificationNeeded,
    CompletionClaimWithoutEvidence,
    TestStatusClaimWithoutCommand,
    PlanClaimDrift,
    RepeatedReasoningFailure,
    ShadowRepeatedPattern,
    CriticUnavailable,
    BriefingUpdatedWithTaskContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningSubject {
    Codebase,
    Documentation,
    Configuration,
    RuntimeStatus,
    TestStatus,
    CompletionStatus,
    ProviderStatus,
    ArchitectureAdvice,
    ExternalFact,
    Plan,
    GeneralReasoning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceSignal {
    Confident,
    Qualified,
    Uncertain,
    ExplicitlySpeculative,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationState {
    NotRequired,
    Unverified,
    PartiallyVerified,
    VerifiedBeforeClaim,
    VerifiedAfterClaim,
    Contradicted,
    CorrectedByUser,
    NeedsHumanReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    FileRead,
    Search,
    Git,
    TestOutput,
    Memory,
    UserGroundTruth,
    ChatHistory,
    PriorVerifiedClaim,
    McpResult,
    PluginResult,
    PipelineArtifact,
    WorldModel,
    PlanStore,
    ProviderTelemetry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeName {
    LearningEvent,
    WorldModel,
    SelfTrust,
    BriefingSummary,
    Retrospective,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataFlowClass {
    Local,
    UserOperated,
    Cloud,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BriefingApplicability {
    CurrentPrompt,
    NextTurn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EvidenceRef {
    pub evidence_id: String,
    pub kind: EvidenceKind,
    pub entity_key: Option<String>,
    pub output_hash: Option<String>,
    pub redacted_excerpt: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Default for EvidenceRef {
    fn default() -> Self {
        Self {
            evidence_id: String::new(),
            kind: EvidenceKind::ChatHistory,
            entity_key: None,
            output_hash: None,
            redacted_excerpt: None,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningClaim {
    pub claim_id: String,
    pub canonicalizer_version: String,
    pub canonical_text: String,
    pub subject: ReasoningSubject,
    pub entity_key: String,
    pub confidence_signal: ConfidenceSignal,
    pub turn_number: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasoningQualityEvent {
    pub event_id: String,
    pub schema_version: u32,
    pub session_id: String,
    pub turn_number: u64,
    pub claim_id: String,
    pub event_kind: ReasoningEventKind,
    pub subject: ReasoningSubject,
    pub entity_key: String,
    pub canonicalizer_version: String,
    pub canonical_text: String,
    pub confidence_signal: ConfidenceSignal,
    pub verification_state: VerificationState,
    pub severity_base: f32,
    pub severity_effective: f32,
    pub severity_override: Option<f32>,
    pub severity_override_reason: Option<String>,
    pub evidence_refs: Vec<EvidenceRef>,
    pub redacted_excerpt: Option<String>,
    pub raw_text_hash: Option<String>,
    pub source_system: String,
    pub shadow: bool,
    pub created_at: DateTime<Utc>,
}

impl Default for ReasoningQualityEvent {
    fn default() -> Self {
        Self {
            event_id: String::new(),
            schema_version: 1,
            session_id: String::new(),
            turn_number: 0,
            claim_id: String::new(),
            event_kind: ReasoningEventKind::ClaimEmitted,
            subject: ReasoningSubject::GeneralReasoning,
            entity_key: String::new(),
            canonicalizer_version: String::new(),
            canonical_text: String::new(),
            confidence_signal: ConfidenceSignal::Unknown,
            verification_state: VerificationState::Unverified,
            severity_base: 0.0,
            severity_effective: 0.0,
            severity_override: None,
            severity_override_reason: None,
            evidence_refs: Vec::new(),
            redacted_excerpt: None,
            raw_text_hash: None,
            source_system: "reasoning_quality".to_string(),
            shadow: true,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasoningTurnInput {
    pub session_id: String,
    pub turn_number: u64,
    pub assistant_text: String,
    pub evidence_refs: Vec<EvidenceRef>,
    pub cwd: Option<String>,
    pub workspace_root: Option<String>,
    pub store_raw_text: bool,
}

impl Default for ReasoningTurnInput {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            turn_number: 0,
            assistant_text: String::new(),
            evidence_refs: Vec::new(),
            cwd: None,
            workspace_root: None,
            store_raw_text: false,
        }
    }
}
