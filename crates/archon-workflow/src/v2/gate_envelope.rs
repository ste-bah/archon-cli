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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deterministic_defect: Option<crate::defect::DeterministicDefect>,
    pub text: String,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    pub remediation_scope: RemediationScope,
    /// The gate stage that produced this finding (#562); see [`GateStage`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_stage: Option<GateStage>,
    /// What the host itself names as decided within `subject`, such as the
    /// obligation a critic judged for a task body (#562); never judge prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judged_item: Option<String>,
}

/// The stage of its gate that produced a finding (#562), in pipeline order
/// within each gate: a later stage sees only what the earlier ones let
/// through. A finding with no stage is from the gate's own validators, or
/// from an envelope written before findings carried one. The fixed
/// decomposition script mirrors these names in `GATE_STAGES` to tell a
/// subject that advances from one that stalls; a test fails if they drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStage {
    /// A model judged an acceptance check and did not accept it.
    Judge,
    /// The host ran a judge-accepted check on the pre-implementation tree.
    Probe,
    /// A check that failed there was judged unable to pass as written.
    Passability,
    /// The obligation fidelity critic found a claimed obligation not established.
    Critic,
}

impl GateStage {
    pub const ALL: [GateStage; 4] = [
        GateStage::Judge,
        GateStage::Probe,
        GateStage::Passability,
        GateStage::Critic,
    ];

    pub fn name(self) -> &'static str {
        match self {
            GateStage::Judge => "judge",
            GateStage::Probe => "probe",
            GateStage::Passability => "passability",
            GateStage::Critic => "critic",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateOperationalError {
    pub kind: String,
    pub text: String,
    /// The subjects the failure left without an outcome, when the producer
    /// knows them (for example the checks an incomplete judge left without
    /// a verdict). Routing reads them here, never from `text`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<String>,
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
