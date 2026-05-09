//! Governed learning models per TSPEC §9.
//!
//! All types for learning events, behaviour proposals, versioned manifests,
//! policy decisions, and approval records.

use serde::{Deserialize, Serialize};

// ── LearningEventType (§9.2) ──────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LearningEventType {
    RetrievalUsed,
    RetrievalRejected,
    SourceConfirmed,
    SourceContradicted,
    AgentRouted,
    AgentOutputUsed,
    AgentOutputRejected,
    GatePassed,
    GateFailed,
    UserAccepted,
    UserCorrected,
    TestPassed,
    TestFailed,
    StrategicRecommendationAccepted,
    StrategicRecommendationRejected,
    CompletionClaimVerified,
    CompletionClaimDowngraded,
    FalseCompletionDetected,
    ManifestApplied,
    ManifestDenied,
    ManifestRolledBack,
    AgentKnowledgeClaim,
}

impl LearningEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RetrievalUsed => "RetrievalUsed",
            Self::RetrievalRejected => "RetrievalRejected",
            Self::SourceConfirmed => "SourceConfirmed",
            Self::SourceContradicted => "SourceContradicted",
            Self::AgentRouted => "AgentRouted",
            Self::AgentOutputUsed => "AgentOutputUsed",
            Self::AgentOutputRejected => "AgentOutputRejected",
            Self::GatePassed => "GatePassed",
            Self::GateFailed => "GateFailed",
            Self::UserAccepted => "UserAccepted",
            Self::UserCorrected => "UserCorrected",
            Self::TestPassed => "TestPassed",
            Self::TestFailed => "TestFailed",
            Self::StrategicRecommendationAccepted => "StrategicRecommendationAccepted",
            Self::StrategicRecommendationRejected => "StrategicRecommendationRejected",
            Self::CompletionClaimVerified => "CompletionClaimVerified",
            Self::CompletionClaimDowngraded => "CompletionClaimDowngraded",
            Self::FalseCompletionDetected => "FalseCompletionDetected",
            Self::ManifestApplied => "ManifestApplied",
            Self::ManifestDenied => "ManifestDenied",
            Self::ManifestRolledBack => "ManifestRolledBack",
            Self::AgentKnowledgeClaim => "AgentKnowledgeClaim",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "RetrievalUsed" => Some(Self::RetrievalUsed),
            "RetrievalRejected" => Some(Self::RetrievalRejected),
            "SourceConfirmed" => Some(Self::SourceConfirmed),
            "SourceContradicted" => Some(Self::SourceContradicted),
            "AgentRouted" => Some(Self::AgentRouted),
            "AgentOutputUsed" => Some(Self::AgentOutputUsed),
            "AgentOutputRejected" => Some(Self::AgentOutputRejected),
            "GatePassed" => Some(Self::GatePassed),
            "GateFailed" => Some(Self::GateFailed),
            "UserAccepted" => Some(Self::UserAccepted),
            "UserCorrected" => Some(Self::UserCorrected),
            "TestPassed" => Some(Self::TestPassed),
            "TestFailed" => Some(Self::TestFailed),
            "StrategicRecommendationAccepted" => Some(Self::StrategicRecommendationAccepted),
            "StrategicRecommendationRejected" => Some(Self::StrategicRecommendationRejected),
            "CompletionClaimVerified" => Some(Self::CompletionClaimVerified),
            "CompletionClaimDowngraded" => Some(Self::CompletionClaimDowngraded),
            "FalseCompletionDetected" => Some(Self::FalseCompletionDetected),
            "ManifestApplied" => Some(Self::ManifestApplied),
            "ManifestDenied" => Some(Self::ManifestDenied),
            "ManifestRolledBack" => Some(Self::ManifestRolledBack),
            "AgentKnowledgeClaim" => Some(Self::AgentKnowledgeClaim),
            _ => None,
        }
    }
}

// ── LearningEvent (§9.1) ──────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LearningEvent {
    pub event_id: String,
    pub workspace_id: String,
    pub event_type: LearningEventType,
    pub source_artifact_id: String,
    pub outcome_artifact_id: Option<String>,
    pub signal: serde_json::Value,
    pub confidence: f32,
    pub provenance_record_id: String,
    pub created_at: String,
}

// ── RiskLevel ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Critical => "Critical",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Low" => Some(Self::Low),
            "Medium" => Some(Self::Medium),
            "High" => Some(Self::High),
            "Critical" => Some(Self::Critical),
            _ => None,
        }
    }

    /// HighRisk or Critical → requires approval. Low/Medium → may auto-apply.
    pub fn is_high_risk(&self) -> bool {
        matches!(self, Self::High | Self::Critical)
    }
}

// ── BehaviourManifestKind (§9.5) ───────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum BehaviourManifestKind {
    RetrievalProfile,
    SourceQualityProfile,
    AgentRoutingProfile,
    ConstellationThresholds,
    PipelineGates,
    BehaviouralRuleAdjustment,
    PromptProfile,
    PolicyOverride,
}

impl BehaviourManifestKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RetrievalProfile => "RetrievalProfile",
            Self::SourceQualityProfile => "SourceQualityProfile",
            Self::AgentRoutingProfile => "AgentRoutingProfile",
            Self::ConstellationThresholds => "ConstellationThresholds",
            Self::PipelineGates => "PipelineGates",
            Self::BehaviouralRuleAdjustment => "BehaviouralRuleAdjustment",
            Self::PromptProfile => "PromptProfile",
            Self::PolicyOverride => "PolicyOverride",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "RetrievalProfile" => Some(Self::RetrievalProfile),
            "SourceQualityProfile" => Some(Self::SourceQualityProfile),
            "AgentRoutingProfile" => Some(Self::AgentRoutingProfile),
            "ConstellationThresholds" => Some(Self::ConstellationThresholds),
            "PipelineGates" => Some(Self::PipelineGates),
            "BehaviouralRuleAdjustment" => Some(Self::BehaviouralRuleAdjustment),
            "PromptProfile" => Some(Self::PromptProfile),
            "PolicyOverride" => Some(Self::PolicyOverride),
            _ => None,
        }
    }

    /// Map manifest kind to its default risk level.
    /// RetrievalProfile/SourceQualityProfile/AgentRoutingProfile/ConstellationThresholds → Low or Medium.
    /// PipelineGates/PromptProfile/PolicyOverride → High or Critical.
    pub fn default_risk_level(&self) -> RiskLevel {
        match self {
            Self::RetrievalProfile
            | Self::SourceQualityProfile
            | Self::AgentRoutingProfile
            | Self::ConstellationThresholds => RiskLevel::Low,
            Self::PipelineGates | Self::BehaviouralRuleAdjustment | Self::PromptProfile => {
                RiskLevel::High
            }
            Self::PolicyOverride => RiskLevel::Critical,
        }
    }
}

// ── PolicyDecision ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PolicyDecision {
    AutoApplied,
    PendingApproval,
    Denied,
    Approved,
    Rejected,
}

impl PolicyDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AutoApplied => "AutoApplied",
            Self::PendingApproval => "PendingApproval",
            Self::Denied => "Denied",
            Self::Approved => "Approved",
            Self::Rejected => "Rejected",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "AutoApplied" => Some(Self::AutoApplied),
            "PendingApproval" => Some(Self::PendingApproval),
            "Denied" => Some(Self::Denied),
            "Approved" => Some(Self::Approved),
            "Rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

// ── ProposalStatus ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProposalStatus {
    Pending,
    Applied,
    Denied,
    RolledBack,
}

impl ProposalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Applied => "Applied",
            Self::Denied => "Denied",
            Self::RolledBack => "RolledBack",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Pending" => Some(Self::Pending),
            "Applied" => Some(Self::Applied),
            "Denied" => Some(Self::Denied),
            "RolledBack" => Some(Self::RolledBack),
            _ => None,
        }
    }
}

// ── BehaviourProposal (§9.3) ──────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BehaviourProposal {
    pub proposal_id: String,
    pub workspace_id: String,
    pub manifest_kind: BehaviourManifestKind,
    pub current_version: String,
    pub proposed_version: String,
    pub diff: String,
    pub evidence_ids: Vec<String>,
    pub risk_level: RiskLevel,
    pub policy_decision: PolicyDecision,
    pub status: ProposalStatus,
    pub created_at: String,
}

// ── BehaviourManifestVersion ───────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BehaviourManifestVersion {
    pub version_id: String,
    pub manifest_kind: BehaviourManifestKind,
    pub version_number: i64,
    pub content: serde_json::Value,
    pub diff: String,
    pub parent_version_id: Option<String>,
    pub created_by_proposal_id: Option<String>,
    pub is_rollback_target: bool,
    pub created_at: String,
}

// ── PolicyOutcome (per-rule decision record) ───────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyOutcome {
    pub rule_name: String,
    pub evaluated: serde_json::Value,
    pub outcome: PolicyDecision,
    pub reason: String,
}

// ── BehaviourApproval ─────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BehaviourApproval {
    pub approval_id: String,
    pub proposal_id: String,
    pub approver: String,
    pub approved: bool,
    pub comment: String,
    pub created_at: String,
}
