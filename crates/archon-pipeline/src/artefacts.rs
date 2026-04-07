//! Structured artefact schemas for pipeline stage handoffs.
//!
//! Implements REQ-IMPROVE-017 (6 typed artefacts) and
//! REQ-IMPROVE-018 (Acceptance Criteria Traced Gate).
//!
//! Artefact chain: TaskContract -> EvidencePack -> WiringPlan ->
//!   ImplementationReport -> ValidationReport -> MergePacket

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

// Re-export earlier artefacts for unified access
pub use crate::coding::contract::TaskContract;
pub use crate::coding::evidence::EvidencePack;
pub use crate::coding::wiring::WiringPlan;

// Also re-export GateResult for the AC traced gate
use crate::coding::contract::GateResult;

// ---------------------------------------------------------------------------
// ImplementationReport
// ---------------------------------------------------------------------------

/// The type of change applied to a file during implementation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ChangeType {
    Created,
    Modified,
    Deleted,
}

/// A single file that was changed during implementation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: String,
    pub change_type: ChangeType,
    pub lines_added: u32,
    pub lines_removed: u32,
}

/// A new symbol introduced into the codebase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewSymbol {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub visibility: String,
}

/// Status of a single wiring obligation after implementation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WiringObligationStatus {
    pub obligation_id: String,
    pub met: bool,
}

/// Produced by: Implementation Coordinator (Phase 4).
///
/// Captures all files changed, new symbols introduced, wiring obligation
/// outcomes, and raw compiler output for audit purposes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImplementationReport {
    pub task_id: String,
    pub changed_files: Vec<ChangedFile>,
    pub new_symbols: Vec<NewSymbol>,
    pub wiring_status: Vec<WiringObligationStatus>,
    pub compiler_output: String,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// ValidationReport
// ---------------------------------------------------------------------------

/// A single gate result entry within a validation report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateResultEntry {
    pub gate_name: String,
    pub passed: bool,
    pub evidence: String,
}

/// Trace record linking an acceptance criterion to its evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceCriterionTrace {
    pub ac_id: String,
    pub description: String,
    /// The source of evidence for this criterion (e.g. test name, log line).
    /// `None` means the criterion is untraced.
    pub evidence_source: Option<String>,
    /// The kind of evidence (e.g. "test_output", "compiler_check").
    pub evidence_type: Option<String>,
}

/// Overall outcome of the validation phase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ValidationStatus {
    AllGatesPassed,
    GatesFailed { failed_gates: Vec<String> },
    UntracedCriteria { untraced: Vec<String> },
}

/// Produced by: Quality Gate (Phase 6).
///
/// Summarises gate outcomes and provides an acceptance-criteria trace
/// so every criterion can be linked to evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub task_id: String,
    pub gate_results: Vec<GateResultEntry>,
    pub overall_status: ValidationStatus,
    pub ac_trace: Vec<AcceptanceCriterionTrace>,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// MergePacket
// ---------------------------------------------------------------------------

/// Risk assessment for a merge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskReport {
    pub risk_level: String,
    pub risk_factors: Vec<String>,
    pub mitigations: Vec<String>,
}

/// An evidence entry binding a gate name to its proof.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceEntry {
    pub gate_name: String,
    pub evidence: String,
}

/// A manual override of a gate, with justification and approver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManualOverrideEntry {
    pub gate_name: String,
    pub justification: String,
    pub overridden_by: String,
}

/// Produced by: Sign-Off Approver (Phase 6).
///
/// The final artefact in the chain, containing everything needed for a
/// reviewer to approve or reject the merge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergePacket {
    pub task_id: String,
    pub summary: String,
    pub risk_report: RiskReport,
    pub evidence_bundle: Vec<EvidenceEntry>,
    pub manual_overrides: Vec<ManualOverrideEntry>,
    pub sign_off_agent: String,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Acceptance Criteria Traced Gate — REQ-IMPROVE-018
// ---------------------------------------------------------------------------

/// Gate that verifies every acceptance criterion from the TaskContract
/// has at least one piece of evidence in the ValidationReport.
pub struct AcceptanceCriteriaTracedGate;

impl AcceptanceCriteriaTracedGate {
    /// Check that every AC has evidence. Returns [`GateResult`].
    ///
    /// A criterion is considered traced when there is an
    /// [`AcceptanceCriterionTrace`] whose `ac_id` matches the criterion's
    /// `id` AND whose `evidence_source` is `Some`.
    pub fn check(contract: &TaskContract, report: &ValidationReport) -> GateResult {
        let mut errors = Vec::new();

        for ac in &contract.acceptance_criteria {
            let traced = report.ac_trace.iter().any(|t| {
                t.ac_id == ac.id && t.evidence_source.is_some()
            });
            if !traced {
                errors.push(format!(
                    "Acceptance criterion '{}' ({}) has no evidence",
                    ac.id, ac.description
                ));
            }
        }

        GateResult {
            passed: errors.is_empty(),
            errors,
        }
    }
}

// ---------------------------------------------------------------------------
// Persistence helpers
// ---------------------------------------------------------------------------

/// Save any serializable artefact to the session directory using an atomic
/// write (write to `.tmp` then rename).
pub fn save_artefact<T: Serialize>(
    artefact: &T,
    filename: &str,
    session_dir: &Path,
) -> Result<()> {
    std::fs::create_dir_all(session_dir)?;
    let path = session_dir.join(filename);
    let json = serde_json::to_string_pretty(artefact)?;
    // Atomic write: write to .tmp then rename
    let tmp_path = session_dir.join(format!("{}.tmp", filename));
    std::fs::write(&tmp_path, &json)?;
    std::fs::rename(&tmp_path, &path)?;
    Ok(())
}

/// Load any deserializable artefact from the session directory.
pub fn load_artefact<T: for<'de> Deserialize<'de>>(
    filename: &str,
    session_dir: &Path,
) -> Result<T> {
    let path = session_dir.join(filename);
    let data = std::fs::read_to_string(&path)?;
    let artefact: T = serde_json::from_str(&data)?;
    Ok(artefact)
}
