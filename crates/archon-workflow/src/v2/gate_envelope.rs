//! Typed wire contract for authoritative decomposition gate output.
//!
//! Gate commands remain the validators. This module only carries their exact
//! report, findings, closed remediation ownership, and operational failure to
//! persisted host-command callers.

use serde::{Deserialize, Serialize};

pub const GATE_ENVELOPE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateEnvelopeV1 {
    pub schema_version: u32,
    pub report: serde_json::Value,
    #[serde(default)]
    pub policy_findings: Vec<GatePolicyFinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operational_error: Option<GateOperationalError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatePolicyFinding {
    pub text: String,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    pub remediation_scope: RemediationScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateOperationalError {
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemediationScope {
    CandidateArtifact,
    Skeleton,
    PrdInput,
    Body,
    InheritedPredecessor,
    Operational,
}
