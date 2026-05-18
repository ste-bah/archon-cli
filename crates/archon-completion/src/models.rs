//! Completion integrity models per TSPEC §10.
//!
//! All types for structured completion claims, evidence, reports,
//! verification gates, false-completion incidents, and trust scores.

use serde::{Deserialize, Serialize};

// ── CompletionState (§10.1) ──────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompletionState {
    Verified,
    Partial,
    Attempted,
    Failed,
    NotRun,
}

// ── CompletionClaim (§10.2) ──────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompletionClaimKind {
    Done,
    Implemented,
    Fixed,
    TestsPass,
    BuildPasses,
    Verified,
    Documented,
    Ingested,
    Indexed,
    AnswerGrounded,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum EvidenceKind {
    CommandRun,
    TestRun,
    BuildResult,
    FileDiff,
    GeneratedArtifact,
    RetrievalEvidence,
    GateResult,
    ReviewFinding,
    CitationTrace,
    IngestionJob,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletionClaim {
    pub claim_id: String,
    pub run_id: String,
    pub agent_key: Option<String>,
    pub model: Option<String>,
    pub task_type: String,
    pub claim_text: String,
    pub claim_kind: CompletionClaimKind,
    pub required_evidence: Vec<EvidenceKind>,
    pub linked_evidence_ids: Vec<String>,
    pub verified: bool,
    pub contradiction_ids: Vec<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletionRunContext {
    pub run_id: String,
    pub workspace_id: String,
    pub agent_key: Option<String>,
    pub model: Option<String>,
    pub updated_at: String,
}

// ── CompletionEvidence (§10.3) ───────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum EvidenceStatus {
    Passed,
    Failed,
    Missing,
    Skipped,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletionEvidence {
    pub evidence_id: String,
    pub run_id: String,
    pub evidence_kind: EvidenceKind,
    pub producer: String,
    pub command_or_operation: Option<String>,
    pub status: EvidenceStatus,
    pub exit_code: Option<i32>,
    pub input_hash: Option<String>,
    pub output_hash: Option<String>,
    pub stdout_summary: Option<String>,
    pub stderr_summary: Option<String>,
    pub artifact_ids: Vec<String>,
    pub provenance_record_id: String,
    pub started_at: String,
    pub completed_at: Option<String>,
}

// ── CompletionReport (§10.4) ─────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletionReport {
    pub report_id: String,
    pub run_id: String,
    pub final_state: CompletionState,
    pub claims: Vec<CompletionClaim>,
    pub evidence: Vec<CompletionEvidence>,
    pub failed_gates: Vec<String>,
    pub unverified_claims: Vec<String>,
    pub contradictions: Vec<String>,
    pub calibrated_summary: String,
    pub provenance_record_id: String,
    pub created_at: String,
}

// ── VerificationGate (§10.5) ─────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerificationGateRequest {
    pub run_id: String,
    pub task_type: String,
    pub claims: Vec<CompletionClaim>,
    pub available_evidence: Vec<CompletionEvidence>,
    pub policy_profile: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerificationGateResult {
    pub gate_id: String,
    pub gate_name: String,
    pub passed: bool,
    pub resulting_state: CompletionState,
    pub blocked_claims: Vec<String>,
    pub required_missing_evidence: Vec<EvidenceKind>,
    pub explanation: String,
    pub provenance_record_id: String,
}

// ── FalseCompletionIncident (§10.6) ──────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum IncidentSeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FalseCompletionIncident {
    pub incident_id: String,
    pub run_id: String,
    pub agent_key: Option<String>,
    pub model: Option<String>,
    pub task_type: String,
    pub claimed_state: String,
    pub actual_state: CompletionState,
    pub missing_evidence: Vec<EvidenceKind>,
    pub contradiction_ids: Vec<String>,
    pub user_correction: Option<String>,
    pub severity: IncidentSeverity,
    pub learning_event_id: String,
    pub created_at: String,
}

// ── AgentModelTrustScore (§10.7) ─────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentModelTrustScore {
    pub score_id: String,
    pub workspace_id: String,
    pub agent_key: Option<String>,
    pub model: Option<String>,
    pub task_type: String,
    pub completion_reliability: f32,
    pub evidence_quality: f32,
    pub false_completion_count: u32,
    pub verified_completion_count: u32,
    pub last_updated: String,
}
