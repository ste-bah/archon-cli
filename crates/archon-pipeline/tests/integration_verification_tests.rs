//! Tests for IntegrationVerifier, VerificationReport, WiringVerificationGate
//! (TASK-PIPE-E04 — REQ-IMPROVE-004, REQ-IMPROVE-007).

use archon_pipeline::coding::wiring::{
    IntegrationVerifier, ObligationVerification, VerificationReport, VerificationStatus,
    WiringAction, WiringObligation, WiringPlan, ObligationStatus,
    WiringVerificationGate,
    save_verification_report, load_verification_report,
};
use archon_pipeline::coding::contract::GateResult;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_plan(task_id: &str, obligations: Vec<WiringObligation>) -> WiringPlan {
    WiringPlan {
        task_id: task_id.into(),
        obligations,
        validated_at: None,
    }
}

fn make_obligation(
    id: &str,
    file: &str,
    action: WiringAction,
    line_context: &str,
    mandatory: bool,
) -> WiringObligation {
    WiringObligation {
        id: id.into(),
        file: file.into(),
        action,
        line_context: line_context.into(),
        mandatory,
        maps_to_contract_wiring: None,
        status: ObligationStatus::Pending,
    }
}

// ---------------------------------------------------------------------------
// Test 1: Verify AddModDecl — present mod declaration
// ---------------------------------------------------------------------------

mod integration_verification {
    use super::*;

    #[test]
    fn verify_mod_decl_present() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let lib_rs = src_dir.join("lib.rs");
        std::fs::write(&lib_rs, "pub mod widgets;\npub mod utils;\n").unwrap();

        let ob = make_obligation(
            "WO-001",
            "src/lib.rs",
            WiringAction::AddModDecl,
            "pub mod widgets;",
            true,
        );
        let plan = make_plan("TEST-001", vec![ob]);
        let report = IntegrationVerifier::verify(&plan, tmp.path());

        assert_eq!(report.results.len(), 1);
        assert_eq!(
            report.results[0].status,
            VerificationStatus::Verified,
            "mod decl is present — should be Verified, evidence: {}",
            report.results[0].evidence
        );
        assert!(report.all_mandatory_met);
    }

    // -----------------------------------------------------------------------
    // Test 2: Verify AddModDecl — missing mod declaration
    // -----------------------------------------------------------------------

    #[test]
    fn verify_mod_decl_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let lib_rs = src_dir.join("lib.rs");
        std::fs::write(&lib_rs, "pub mod utils;\n").unwrap(); // widgets is NOT here

        let ob = make_obligation(
            "WO-001",
            "src/lib.rs",
            WiringAction::AddModDecl,
            "pub mod widgets;",
            true,
        );
        let plan = make_plan("TEST-001", vec![ob]);
        let report = IntegrationVerifier::verify(&plan, tmp.path());

        assert_eq!(
            report.results[0].status,
            VerificationStatus::Unmet,
            "mod decl is missing — should be Unmet"
        );
        assert!(!report.all_mandatory_met);
    }

    // -----------------------------------------------------------------------
    // Test 3: Verify AddImport — use statement present
    // -----------------------------------------------------------------------

    #[test]
    fn verify_import_present() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let handler = src_dir.join("handler.rs");
        std::fs::write(
            &handler,
            "use crate::widgets::Button;\n\nfn render() {}\n",
        )
        .unwrap();

        let ob = make_obligation(
            "WO-002",
            "src/handler.rs",
            WiringAction::AddImport,
            "use crate::widgets::Button",
            true,
        );
        let plan = make_plan("TEST-001", vec![ob]);
        let report = IntegrationVerifier::verify(&plan, tmp.path());

        assert_eq!(
            report.results[0].status,
            VerificationStatus::Verified,
            "import is present — should be Verified"
        );
        assert!(report.all_mandatory_met);
    }

    // -----------------------------------------------------------------------
    // Test 4: Verify AddSerdeDerive — both Serialize and Deserialize present
    // -----------------------------------------------------------------------

    #[test]
    fn verify_serde_derive_present() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let model = src_dir.join("model.rs");
        std::fs::write(
            &model,
            "#[derive(Debug, Clone, Serialize, Deserialize)]\npub struct Config {\n    pub name: String,\n}\n",
        )
        .unwrap();

        let ob = make_obligation(
            "WO-003",
            "src/model.rs",
            WiringAction::AddSerdeDerive,
            "#[derive(Serialize, Deserialize)]",
            true,
        );
        let plan = make_plan("TEST-001", vec![ob]);
        let report = IntegrationVerifier::verify(&plan, tmp.path());

        assert_eq!(
            report.results[0].status,
            VerificationStatus::Verified,
            "serde derive is present — should be Verified"
        );
        assert!(report.all_mandatory_met);
    }

    // -----------------------------------------------------------------------
    // Test 5: Verify RegisterRoute — route registration pattern
    // -----------------------------------------------------------------------

    #[test]
    fn verify_register_route_present() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let router = src_dir.join("router.rs");
        std::fs::write(
            &router,
            "pub fn build_router() -> Router {\n    Router::new().route(\"/health\", get(health_handler))\n}\n",
        )
        .unwrap();

        let ob = make_obligation(
            "WO-004",
            "src/router.rs",
            WiringAction::RegisterRoute,
            ".route(\"/health\", get(health_handler))",
            true,
        );
        let plan = make_plan("TEST-001", vec![ob]);
        let report = IntegrationVerifier::verify(&plan, tmp.path());

        assert_eq!(
            report.results[0].status,
            VerificationStatus::Verified,
            "route registration is present — should be Verified"
        );
        assert!(report.all_mandatory_met);
    }

    // -----------------------------------------------------------------------
    // Test 6: WiringVerificationGate passes when all mandatory obligations Met
    // -----------------------------------------------------------------------

    #[test]
    fn gate_passes_when_all_mandatory_met() {
        let report = VerificationReport {
            task_id: "TEST-001".into(),
            results: vec![
                ObligationVerification {
                    obligation_id: "WO-001".into(),
                    status: VerificationStatus::Verified,
                    evidence: "pub mod widgets;".into(),
                    tool_used: "Read".into(),
                },
                ObligationVerification {
                    obligation_id: "WO-002".into(),
                    status: VerificationStatus::Verified,
                    evidence: "use crate::widgets;".into(),
                    tool_used: "Read".into(),
                },
            ],
            all_mandatory_met: true,
            verified_at: "1234567890Z".into(),
        };

        let gate: GateResult = WiringVerificationGate::check(&report);
        assert!(gate.passed, "gate should pass: {:?}", gate.errors);
        assert!(gate.errors.is_empty());
    }

    // -----------------------------------------------------------------------
    // Test 7: WiringVerificationGate fails when any mandatory obligation Unmet
    // -----------------------------------------------------------------------

    #[test]
    fn gate_fails_when_mandatory_unmet() {
        let report = VerificationReport {
            task_id: "TEST-001".into(),
            results: vec![
                ObligationVerification {
                    obligation_id: "WO-001".into(),
                    status: VerificationStatus::Unmet,
                    evidence: "pattern not found".into(),
                    tool_used: "Read".into(),
                },
            ],
            all_mandatory_met: false,
            verified_at: "1234567890Z".into(),
        };

        let gate: GateResult = WiringVerificationGate::check(&report);
        assert!(!gate.passed, "gate should fail when mandatory obligation unmet");
        assert!(!gate.errors.is_empty());
        assert!(gate.errors[0].contains("WO-001"));
    }

    // -----------------------------------------------------------------------
    // Test 8: Skipped (non-mandatory) obligations don't affect gate pass/fail
    // -----------------------------------------------------------------------

    #[test]
    fn skipped_non_mandatory_does_not_affect_gate() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let lib_rs = src_dir.join("lib.rs");
        std::fs::write(&lib_rs, "pub mod widgets;\n").unwrap();

        let mandatory_ob = make_obligation(
            "WO-001",
            "src/lib.rs",
            WiringAction::AddModDecl,
            "pub mod widgets;",
            true,
        );
        let optional_ob = make_obligation(
            "WO-002",
            "src/lib.rs",
            WiringAction::AddImport,
            "use crate::something_optional;",
            false, // NOT mandatory
        );
        let plan = make_plan("TEST-001", vec![mandatory_ob, optional_ob]);
        let report = IntegrationVerifier::verify(&plan, tmp.path());

        assert_eq!(
            report.results[1].status,
            VerificationStatus::Skipped,
            "non-mandatory should be Skipped"
        );
        assert!(
            report.all_mandatory_met,
            "all mandatory obligations should still be met"
        );

        let gate: GateResult = WiringVerificationGate::check(&report);
        assert!(gate.passed, "gate should pass despite skipped non-mandatory: {:?}", gate.errors);
    }

    // -----------------------------------------------------------------------
    // Test 9: VerificationReport serializes/deserializes correctly
    // -----------------------------------------------------------------------

    #[test]
    fn verification_report_serialization_roundtrip() {
        let report = VerificationReport {
            task_id: "TEST-SERDE".into(),
            results: vec![ObligationVerification {
                obligation_id: "WO-001".into(),
                status: VerificationStatus::Verified,
                evidence: "found pub mod x;".into(),
                tool_used: "Read".into(),
            }],
            all_mandatory_met: true,
            verified_at: "9999999999Z".into(),
        };

        let json = serde_json::to_string_pretty(&report).unwrap();
        let loaded: VerificationReport = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.task_id, "TEST-SERDE");
        assert_eq!(loaded.results.len(), 1);
        assert_eq!(loaded.results[0].obligation_id, "WO-001");
        assert_eq!(loaded.results[0].status, VerificationStatus::Verified);
        assert!(loaded.all_mandatory_met);
    }

    // -----------------------------------------------------------------------
    // Test 10: VerificationReport persists to and loads from session dir
    // -----------------------------------------------------------------------

    #[test]
    fn verification_report_persistence_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let session_dir = tmp.path().join(".pipeline-state").join("session-abc");

        let report = VerificationReport {
            task_id: "TEST-PERSIST".into(),
            results: vec![ObligationVerification {
                obligation_id: "WO-001".into(),
                status: VerificationStatus::Unmet,
                evidence: "pattern not found in file".into(),
                tool_used: "Read".into(),
            }],
            all_mandatory_met: false,
            verified_at: "1111111111Z".into(),
        };

        save_verification_report(&report, &session_dir).unwrap();

        let report_path = session_dir.join("verification-report.json");
        assert!(report_path.exists(), "verification-report.json should exist");

        let loaded = load_verification_report(&session_dir).unwrap();
        assert_eq!(loaded.task_id, "TEST-PERSIST");
        assert_eq!(loaded.results[0].status, VerificationStatus::Unmet);
        assert!(!loaded.all_mandatory_met);
    }
}
