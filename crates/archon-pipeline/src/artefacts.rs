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
            let traced = report
                .ac_trace
                .iter()
                .any(|t| t.ac_id == ac.id && t.evidence_source.is_some());
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
pub fn save_artefact<T: Serialize>(artefact: &T, filename: &str, session_dir: &Path) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::contract::{AcceptanceCriterion, TaskContract};

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn sample_implementation_report() -> ImplementationReport {
        ImplementationReport {
            task_id: "TASK-001".into(),
            changed_files: vec![ChangedFile {
                path: "src/lib.rs".into(),
                change_type: ChangeType::Modified,
                lines_added: 10,
                lines_removed: 2,
            }],
            new_symbols: vec![NewSymbol {
                name: "foo".into(),
                kind: "function".into(),
                file: "src/lib.rs".into(),
                line: 42,
                visibility: "pub".into(),
            }],
            wiring_status: vec![WiringObligationStatus {
                obligation_id: "W-1".into(),
                met: true,
            }],
            compiler_output: "Compiling ok".into(),
            created_at: "2026-04-08T00:00:00Z".into(),
        }
    }

    fn sample_validation_report(status: ValidationStatus) -> ValidationReport {
        ValidationReport {
            task_id: "TASK-001".into(),
            gate_results: vec![GateResultEntry {
                gate_name: "compile".into(),
                passed: true,
                evidence: "exit 0".into(),
            }],
            overall_status: status,
            ac_trace: vec![AcceptanceCriterionTrace {
                ac_id: "AC-1".into(),
                description: "It works".into(),
                evidence_source: Some("test_it_works".into()),
                evidence_type: Some("test_output".into()),
            }],
            created_at: "2026-04-08T00:00:00Z".into(),
        }
    }

    fn sample_merge_packet() -> MergePacket {
        MergePacket {
            task_id: "TASK-001".into(),
            summary: "Added foo".into(),
            risk_report: RiskReport {
                risk_level: "low".into(),
                risk_factors: vec!["none".into()],
                mitigations: vec!["tests".into()],
            },
            evidence_bundle: vec![EvidenceEntry {
                gate_name: "compile".into(),
                evidence: "ok".into(),
            }],
            manual_overrides: vec![],
            sign_off_agent: "approver-v1".into(),
            created_at: "2026-04-08T00:00:00Z".into(),
        }
    }

    fn sample_task_contract(ac_ids: &[&str]) -> TaskContract {
        TaskContract {
            task_id: "TASK-001".into(),
            goal: "do things".into(),
            non_goals: vec![],
            acceptance_criteria: ac_ids
                .iter()
                .map(|id| AcceptanceCriterion {
                    id: id.to_string(),
                    description: format!("criterion {}", id),
                    verification: "test".into(),
                })
                .collect(),
            affected_files: vec![],
            required_wiring: vec![],
            required_tests: vec![],
            rollback_plan: "revert".into(),
            definition_of_done: vec!["done".into()],
        }
    }

    // -----------------------------------------------------------------------
    // Serde roundtrip tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_implementation_report_serde_roundtrip() {
        let report = sample_implementation_report();
        let json = serde_json::to_string(&report).unwrap();
        let deserialized: ImplementationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report, deserialized);
    }

    #[test]
    fn test_validation_report_serde_roundtrip() {
        let report = sample_validation_report(ValidationStatus::AllGatesPassed);
        let json = serde_json::to_string(&report).unwrap();
        let deserialized: ValidationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report, deserialized);
    }

    #[test]
    fn test_merge_packet_serde_roundtrip() {
        let packet = sample_merge_packet();
        let json = serde_json::to_string(&packet).unwrap();
        let deserialized: MergePacket = serde_json::from_str(&json).unwrap();
        assert_eq!(packet, deserialized);
    }

    #[test]
    fn test_validation_status_gates_failed() {
        let report = sample_validation_report(ValidationStatus::GatesFailed {
            failed_gates: vec!["lint".into(), "clippy".into()],
        });
        let json = serde_json::to_string(&report).unwrap();
        let deserialized: ValidationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report, deserialized);
    }

    // -----------------------------------------------------------------------
    // Persistence tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_save_and_load_artefact() {
        let dir = tempfile::tempdir().unwrap();
        let report = sample_implementation_report();
        save_artefact(&report, "impl.json", dir.path()).unwrap();
        let loaded: ImplementationReport = load_artefact("impl.json", dir.path()).unwrap();
        assert_eq!(report, loaded);
    }

    #[test]
    fn test_save_artefact_creates_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("c");
        let report = sample_implementation_report();
        save_artefact(&report, "impl.json", &nested).unwrap();
        assert!(nested.join("impl.json").exists());
    }

    #[test]
    fn test_load_artefact_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let result: Result<ImplementationReport> = load_artefact("nope.json", dir.path());
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // AC Traced Gate tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_ac_traced_gate_all_traced() {
        let contract = sample_task_contract(&["AC-1", "AC-2"]);
        let report = ValidationReport {
            task_id: "TASK-001".into(),
            gate_results: vec![],
            overall_status: ValidationStatus::AllGatesPassed,
            ac_trace: vec![
                AcceptanceCriterionTrace {
                    ac_id: "AC-1".into(),
                    description: "criterion AC-1".into(),
                    evidence_source: Some("test_ac1".into()),
                    evidence_type: Some("test_output".into()),
                },
                AcceptanceCriterionTrace {
                    ac_id: "AC-2".into(),
                    description: "criterion AC-2".into(),
                    evidence_source: Some("test_ac2".into()),
                    evidence_type: Some("test_output".into()),
                },
            ],
            created_at: "2026-04-08T00:00:00Z".into(),
        };

        let result = AcceptanceCriteriaTracedGate::check(&contract, &report);
        assert!(result.passed);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_ac_traced_gate_missing_evidence() {
        let contract = sample_task_contract(&["AC-1", "AC-2"]);
        let report = ValidationReport {
            task_id: "TASK-001".into(),
            gate_results: vec![],
            overall_status: ValidationStatus::AllGatesPassed,
            ac_trace: vec![AcceptanceCriterionTrace {
                ac_id: "AC-1".into(),
                description: "criterion AC-1".into(),
                evidence_source: Some("test_ac1".into()),
                evidence_type: Some("test_output".into()),
            }],
            created_at: "2026-04-08T00:00:00Z".into(),
        };

        let result = AcceptanceCriteriaTracedGate::check(&contract, &report);
        assert!(!result.passed);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].contains("AC-2"));
    }
}
