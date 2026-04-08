//! Tests for structured artefact schemas — REQ-IMPROVE-017, REQ-IMPROVE-018.

use archon_pipeline::artefacts::{
    AcceptanceCriteriaTracedGate,
    AcceptanceCriterionTrace,
    ChangeType,
    ChangedFile,
    EvidenceEntry,
    // Re-exported earlier artefacts
    EvidencePack,
    GateResultEntry,
    ImplementationReport,
    ManualOverrideEntry,
    MergePacket,
    NewSymbol,
    RiskReport,
    TaskContract,
    ValidationReport,
    ValidationStatus,
    WiringObligationStatus,
    WiringPlan,
};
use archon_pipeline::artefacts::{load_artefact, save_artefact};
use archon_pipeline::coding::contract::AcceptanceCriterion;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_implementation_report() -> ImplementationReport {
    ImplementationReport {
        task_id: "TASK-001".into(),
        changed_files: vec![ChangedFile {
            path: "src/lib.rs".into(),
            change_type: ChangeType::Modified,
            lines_added: 42,
            lines_removed: 10,
        }],
        new_symbols: vec![NewSymbol {
            name: "my_fn".into(),
            kind: "function".into(),
            file: "src/lib.rs".into(),
            line: 100,
            visibility: "pub".into(),
        }],
        wiring_status: vec![WiringObligationStatus {
            obligation_id: "OB-001".into(),
            met: true,
        }],
        compiler_output: "Compiling archon-pipeline v0.1.0\n   Finished".into(),
        created_at: "2026-04-07T00:00:00Z".into(),
    }
}

fn make_validation_report(with_evidence: bool) -> ValidationReport {
    ValidationReport {
        task_id: "TASK-001".into(),
        gate_results: vec![GateResultEntry {
            gate_name: "tests-passing".into(),
            passed: true,
            evidence: "12 tests passed".into(),
        }],
        overall_status: ValidationStatus::AllGatesPassed,
        ac_trace: vec![AcceptanceCriterionTrace {
            ac_id: "AC-001".into(),
            description: "Serializes without loss".into(),
            evidence_source: if with_evidence {
                Some("cargo test output line 42".into())
            } else {
                None
            },
            evidence_type: if with_evidence {
                Some("test_output".into())
            } else {
                None
            },
        }],
        created_at: "2026-04-07T00:00:00Z".into(),
    }
}

fn make_merge_packet() -> MergePacket {
    MergePacket {
        task_id: "TASK-001".into(),
        summary: "Implemented artefact schemas".into(),
        risk_report: RiskReport {
            risk_level: "low".into(),
            risk_factors: vec!["New public API".into()],
            mitigations: vec!["Full test coverage".into()],
        },
        evidence_bundle: vec![EvidenceEntry {
            gate_name: "tests-passing".into(),
            evidence: "12/12 tests passed".into(),
        }],
        manual_overrides: vec![ManualOverrideEntry {
            gate_name: "live-smoke-test".into(),
            justification: "Unit-test-only feature, no CLI hook yet".into(),
            overridden_by: "sign-off-approver".into(),
        }],
        sign_off_agent: "sign-off-approver-v1".into(),
        created_at: "2026-04-07T00:00:00Z".into(),
    }
}

fn make_task_contract(ac_ids: &[&str]) -> TaskContract {
    TaskContract {
        task_id: "TASK-001".into(),
        goal: "Test goal".into(),
        non_goals: vec![],
        acceptance_criteria: ac_ids
            .iter()
            .map(|id| AcceptanceCriterion {
                id: id.to_string(),
                description: format!("Criterion {}", id),
                verification: "automated test".into(),
            })
            .collect(),
        affected_files: vec![],
        required_wiring: vec![],
        required_tests: vec![],
        rollback_plan: "Revert commit".into(),
        definition_of_done: vec!["All tests pass".into()],
    }
}

// ---------------------------------------------------------------------------
// 1. ImplementationReport serializes to JSON and deserializes without loss
// ---------------------------------------------------------------------------

#[test]
fn test_implementation_report_serde_roundtrip() {
    let report = make_implementation_report();
    let json = serde_json::to_string_pretty(&report).expect("serialize");
    let restored: ImplementationReport = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(report, restored);
}

// ---------------------------------------------------------------------------
// 2. ValidationReport serializes/deserializes correctly
// ---------------------------------------------------------------------------

#[test]
fn test_validation_report_serde_roundtrip() {
    let report = make_validation_report(true);
    let json = serde_json::to_string_pretty(&report).expect("serialize");
    let restored: ValidationReport = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(report, restored);
}

// ---------------------------------------------------------------------------
// 3. MergePacket serializes/deserializes correctly
// ---------------------------------------------------------------------------

#[test]
fn test_merge_packet_serde_roundtrip() {
    let packet = make_merge_packet();
    let json = serde_json::to_string_pretty(&packet).expect("serialize");
    let restored: MergePacket = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(packet, restored);
}

// ---------------------------------------------------------------------------
// 4. AC Traced Gate passes when all criteria have evidence
// ---------------------------------------------------------------------------

#[test]
fn test_ac_traced_gate_passes_when_all_traced() {
    let contract = make_task_contract(&["AC-001"]);
    let report = make_validation_report(true); // AC-001 has evidence
    let result = AcceptanceCriteriaTracedGate::check(&contract, &report);
    assert!(result.passed, "Gate should pass when all ACs have evidence");
    assert!(result.errors.is_empty());
}

// ---------------------------------------------------------------------------
// 5. AC Traced Gate fails when a criterion has no evidence
// ---------------------------------------------------------------------------

#[test]
fn test_ac_traced_gate_fails_when_untraced() {
    let contract = make_task_contract(&["AC-001"]);
    let report = make_validation_report(false); // AC-001 has no evidence (None)
    let result = AcceptanceCriteriaTracedGate::check(&contract, &report);
    assert!(
        !result.passed,
        "Gate should fail when an AC has no evidence"
    );
    assert!(!result.errors.is_empty());
}

// ---------------------------------------------------------------------------
// 6. AC Traced Gate returns list of untraced criteria IDs in error
// ---------------------------------------------------------------------------

#[test]
fn test_ac_traced_gate_reports_untraced_ids() {
    let contract = make_task_contract(&["AC-001", "AC-002"]);
    // Report only traces AC-001 with evidence; AC-002 is absent entirely
    let report = ValidationReport {
        task_id: "TASK-001".into(),
        gate_results: vec![],
        overall_status: ValidationStatus::AllGatesPassed,
        ac_trace: vec![AcceptanceCriterionTrace {
            ac_id: "AC-001".into(),
            description: "Criterion AC-001".into(),
            evidence_source: Some("test output".into()),
            evidence_type: Some("test_output".into()),
        }],
        created_at: "2026-04-07T00:00:00Z".into(),
    };
    let result = AcceptanceCriteriaTracedGate::check(&contract, &report);
    assert!(!result.passed);
    // Should mention AC-002
    assert!(
        result.errors.iter().any(|e| e.contains("AC-002")),
        "Expected error message to mention AC-002, got: {:?}",
        result.errors
    );
}

// ---------------------------------------------------------------------------
// 7. All 6 artefact types are importable from archon_pipeline::artefacts
// ---------------------------------------------------------------------------

#[test]
fn test_all_six_artefact_types_importable() {
    // TaskContract, EvidencePack, WiringPlan are re-exported
    let _: fn() -> TaskContract = || make_task_contract(&[]);
    // ImplementationReport, ValidationReport, MergePacket are new types
    let _: fn() -> ImplementationReport = make_implementation_report;
    let _: fn() -> ValidationReport = || make_validation_report(true);
    let _: fn() -> MergePacket = make_merge_packet;

    // EvidencePack: just construct a minimal one
    let _pack = EvidencePack {
        facts: vec![],
        call_graph: archon_pipeline::coding::evidence::CallGraph { edges: vec![] },
        existing_tests: vec![],
        entrypoints: vec![],
        api_contracts: vec![],
    };

    // WiringPlan: construct a minimal one
    let _plan = WiringPlan {
        task_id: "T".into(),
        obligations: vec![],
        validated_at: None,
    };
}

// ---------------------------------------------------------------------------
// 8. Artefact persistence: save to dir, reload, compare equality
// ---------------------------------------------------------------------------

#[test]
fn test_artefact_persistence_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = make_implementation_report();

    save_artefact(&report, "impl-report.json", dir.path()).expect("save");
    let loaded: ImplementationReport = load_artefact("impl-report.json", dir.path()).expect("load");

    assert_eq!(report, loaded);
}

#[test]
fn test_validation_report_persistence_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = make_validation_report(true);

    save_artefact(&report, "validation-report.json", dir.path()).expect("save");
    let loaded: ValidationReport =
        load_artefact("validation-report.json", dir.path()).expect("load");

    assert_eq!(report, loaded);
}

#[test]
fn test_merge_packet_persistence_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let packet = make_merge_packet();

    save_artefact(&packet, "merge-packet.json", dir.path()).expect("save");
    let loaded: MergePacket = load_artefact("merge-packet.json", dir.path()).expect("load");

    assert_eq!(packet, loaded);
}

// ---------------------------------------------------------------------------
// 9. MergePacket includes manual_overrides field
// ---------------------------------------------------------------------------

#[test]
fn test_merge_packet_has_manual_overrides() {
    let packet = make_merge_packet();
    assert_eq!(packet.manual_overrides.len(), 1);
    assert_eq!(packet.manual_overrides[0].gate_name, "live-smoke-test");
    assert_eq!(
        packet.manual_overrides[0].overridden_by,
        "sign-off-approver"
    );

    // Verify it round-trips through JSON preserving the field
    let json = serde_json::to_string(&packet).expect("serialize");
    assert!(
        json.contains("manual_overrides"),
        "JSON must contain manual_overrides key"
    );
    let restored: MergePacket = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored.manual_overrides, packet.manual_overrides);
}
