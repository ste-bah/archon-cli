//! Tests for Wiring Obligation Agent + WiringPlan (TASK-PIPE-E03).
//!
//! Validates: WiringPlan schema, obligation status tracking,
//! coverage validation against TaskContract, persistence, and gate checks.

use archon_pipeline::coding::contract::{TaskContract, AcceptanceCriterion, WiringRequirement, WiringType, TestRequirement, TestType};
use archon_pipeline::coding::wiring::{
    WiringPlan, WiringObligation, WiringAction, ObligationStatus,
    WiringPlanApprovedGate, validate_coverage, save_wiring_plan, load_wiring_plan,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sample_contract() -> TaskContract {
    TaskContract {
        task_id: "TEST-001".into(),
        goal: "Add user profile page".into(),
        non_goals: vec![],
        acceptance_criteria: vec![AcceptanceCriterion {
            id: "AC-001".into(),
            description: "Profile page renders".into(),
            verification: "Manual".into(),
        }],
        affected_files: vec!["src/pages/profile.rs".into()],
        required_wiring: vec![
            WiringRequirement {
                module: "src/router.rs".into(),
                entrypoint: "register_routes".into(),
                wiring_type: WiringType::RouteRegistration,
            },
            WiringRequirement {
                module: "src/lib.rs".into(),
                entrypoint: "mod pages".into(),
                wiring_type: WiringType::ModDeclaration,
            },
        ],
        required_tests: vec![TestRequirement {
            test_type: TestType::Unit,
            description: "Profile page tests".into(),
        }],
        rollback_plan: "Revert commit".into(),
        definition_of_done: vec!["Page renders".into()],
    }
}

fn sample_plan(task_id: &str) -> WiringPlan {
    WiringPlan {
        task_id: task_id.into(),
        obligations: vec![
            WiringObligation {
                id: "WO-001".into(),
                file: "src/router.rs".into(),
                action: WiringAction::RegisterRoute,
                line_context: "router.route(\"/profile\", profile_handler)".into(),
                mandatory: true,
                maps_to_contract_wiring: Some("src/router.rs::register_routes".into()),
                status: ObligationStatus::Pending,
            },
            WiringObligation {
                id: "WO-002".into(),
                file: "src/lib.rs".into(),
                action: WiringAction::AddModDecl,
                line_context: "pub mod pages;".into(),
                mandatory: true,
                maps_to_contract_wiring: Some("src/lib.rs::mod pages".into()),
                status: ObligationStatus::Pending,
            },
        ],
        validated_at: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

mod wiring_tests {
    use super::*;

    #[test]
    fn wiring_plan_serialization_roundtrip() {
        let plan = sample_plan("TEST-001");
        let json = serde_json::to_string_pretty(&plan).unwrap();
        let deserialized: WiringPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.task_id, "TEST-001");
        assert_eq!(deserialized.obligations.len(), 2);
        assert_eq!(deserialized.obligations[0].id, "WO-001");
        assert_eq!(deserialized.obligations[1].status, ObligationStatus::Pending);
    }

    #[test]
    fn obligation_status_transitions() {
        let mut obligation = WiringObligation {
            id: "WO-001".into(),
            file: "src/main.rs".into(),
            action: WiringAction::AddImport,
            line_context: "use crate::profile;".into(),
            mandatory: true,
            maps_to_contract_wiring: None,
            status: ObligationStatus::Pending,
        };

        assert_eq!(obligation.status, ObligationStatus::Pending);
        obligation.status = ObligationStatus::Met;
        assert_eq!(obligation.status, ObligationStatus::Met);
        obligation.status = ObligationStatus::Failed;
        assert_eq!(obligation.status, ObligationStatus::Failed);
    }

    #[test]
    fn wiring_action_variants() {
        let actions = vec![
            WiringAction::AddModDecl,
            WiringAction::AddImport,
            WiringAction::RegisterRoute,
            WiringAction::AddConfigKey,
            WiringAction::AddSerdeDerive,
            WiringAction::Other("custom wiring".into()),
        ];

        // All should serialize/deserialize without error
        for action in &actions {
            let json = serde_json::to_string(action).unwrap();
            let _: WiringAction = serde_json::from_str(&json).unwrap();
        }
    }

    #[test]
    fn validate_coverage_passes_when_all_wiring_covered() {
        let contract = sample_contract();
        let plan = sample_plan("TEST-001");

        let result = validate_coverage(&contract, &plan);
        assert!(result.is_ok(), "all wiring should be covered: {:?}", result.err());
    }

    #[test]
    fn validate_coverage_fails_for_missing_obligation() {
        let contract = sample_contract();
        // Plan with only one obligation — missing the ModDeclaration wiring
        let plan = WiringPlan {
            task_id: "TEST-001".into(),
            obligations: vec![WiringObligation {
                id: "WO-001".into(),
                file: "src/router.rs".into(),
                action: WiringAction::RegisterRoute,
                line_context: "router.route(...)".into(),
                mandatory: true,
                maps_to_contract_wiring: Some("src/router.rs::register_routes".into()),
                status: ObligationStatus::Pending,
            }],
            validated_at: None,
        };

        let result = validate_coverage(&contract, &plan);
        assert!(result.is_err(), "should fail when wiring requirement has no matching obligation");
        let errors = result.unwrap_err();
        assert!(!errors.is_empty());
    }

    #[test]
    fn validate_coverage_ok_for_empty_contract_wiring() {
        let mut contract = sample_contract();
        contract.required_wiring.clear();
        let plan = WiringPlan {
            task_id: "TEST-001".into(),
            obligations: vec![],
            validated_at: None,
        };

        let result = validate_coverage(&contract, &plan);
        assert!(result.is_ok(), "empty wiring requirements should pass");
    }

    #[test]
    fn gate_check_passes_with_full_coverage() {
        let contract = sample_contract();
        let plan = sample_plan("TEST-001");

        let result = WiringPlanApprovedGate::check(&contract, &plan);
        assert!(result.passed, "gate should pass: {:?}", result.errors);
    }

    #[test]
    fn gate_check_fails_with_missing_coverage() {
        let contract = sample_contract();
        let plan = WiringPlan {
            task_id: "TEST-001".into(),
            obligations: vec![],
            validated_at: None,
        };

        let result = WiringPlanApprovedGate::check(&contract, &plan);
        assert!(!result.passed, "gate should fail with no obligations covering contract wiring");
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn gate_check_fails_with_empty_file_in_obligation() {
        let mut contract = sample_contract();
        contract.required_wiring.clear();
        let plan = WiringPlan {
            task_id: "TEST-001".into(),
            obligations: vec![WiringObligation {
                id: "WO-001".into(),
                file: "".into(), // empty file — invalid
                action: WiringAction::AddModDecl,
                line_context: "pub mod x;".into(),
                mandatory: true,
                maps_to_contract_wiring: None,
                status: ObligationStatus::Pending,
            }],
            validated_at: None,
        };

        let result = WiringPlanApprovedGate::check(&contract, &plan);
        assert!(!result.passed, "gate should fail with empty file");
    }

    #[test]
    fn gate_check_fails_with_empty_line_context() {
        let mut contract = sample_contract();
        contract.required_wiring.clear();
        let plan = WiringPlan {
            task_id: "TEST-001".into(),
            obligations: vec![WiringObligation {
                id: "WO-001".into(),
                file: "src/main.rs".into(),
                action: WiringAction::AddImport,
                line_context: "".into(), // empty — invalid
                mandatory: true,
                maps_to_contract_wiring: None,
                status: ObligationStatus::Pending,
            }],
            validated_at: None,
        };

        let result = WiringPlanApprovedGate::check(&contract, &plan);
        assert!(!result.passed, "gate should fail with empty line_context");
    }

    #[test]
    fn persistence_roundtrip() {
        let plan = sample_plan("TEST-001");
        let tmp = tempfile::tempdir().unwrap();

        save_wiring_plan(&plan, tmp.path()).unwrap();
        let loaded = load_wiring_plan(tmp.path()).unwrap();

        assert_eq!(loaded.task_id, plan.task_id);
        assert_eq!(loaded.obligations.len(), plan.obligations.len());
        assert_eq!(loaded.obligations[0].id, "WO-001");
        assert_eq!(loaded.obligations[1].action, WiringAction::AddModDecl);
    }

    #[test]
    fn load_missing_file_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load_wiring_plan(tmp.path());
        assert!(result.is_err(), "loading from empty dir should fail");
    }

    #[test]
    fn wiring_obligation_agent_in_phase_3() {
        use archon_pipeline::coding::{AGENTS, Phase};
        let wiring_agent = AGENTS.iter().find(|a| a.key == "wiring-obligation-agent");
        assert!(wiring_agent.is_some(), "wiring-obligation-agent should exist in AGENTS");
        let agent = wiring_agent.unwrap();
        assert_eq!(agent.phase, Phase::WiringPlan, "should be Phase 3 WiringPlan");
        assert!(agent.critical, "should be critical");
    }
}
