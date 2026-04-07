//! Tests for TaskContract schema, validation, gate, and alternatives.

use archon_pipeline::coding::contract::{
    AcceptanceCriterion, ContractValidatedGate, TaskContract, TestRequirement, TestType,
    WiringRequirement, WiringType, generate_alternatives, load_contract, save_contract,
    validate_contract,
};
use archon_pipeline::coding::agents::AGENTS;
use tempfile::TempDir;

/// Helper: build a fully valid TaskContract.
fn valid_contract() -> TaskContract {
    TaskContract {
        task_id: "TASK-001".into(),
        goal: "Implement contract validation".into(),
        non_goals: vec!["Do not touch the UI".into()],
        acceptance_criteria: vec![AcceptanceCriterion {
            id: "AC-001".into(),
            description: "Contract validates successfully".into(),
            verification: "Unit test passes".into(),
        }],
        affected_files: vec!["src/coding/contract.rs".into()],
        required_wiring: vec![WiringRequirement {
            module: "coding".into(),
            entrypoint: "mod.rs".into(),
            wiring_type: WiringType::ModDeclaration,
        }],
        required_tests: vec![TestRequirement {
            test_type: TestType::Unit,
            description: "Validate contract fields".into(),
        }],
        rollback_plan: "Revert commit".into(),
        definition_of_done: vec!["All tests pass".into()],
    }
}

// ---------------------------------------------------------------------------
// 1. Valid contract serializes/deserializes correctly
// ---------------------------------------------------------------------------

#[test]
fn valid_contract_serde_roundtrip() {
    let contract = valid_contract();
    let json = serde_json::to_string_pretty(&contract).expect("serialize");
    let deser: TaskContract = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deser.task_id, contract.task_id);
    assert_eq!(deser.goal, contract.goal);
    assert_eq!(deser.non_goals.len(), contract.non_goals.len());
    assert_eq!(deser.acceptance_criteria.len(), contract.acceptance_criteria.len());
    assert_eq!(deser.affected_files.len(), contract.affected_files.len());
    assert_eq!(deser.required_wiring.len(), contract.required_wiring.len());
    assert_eq!(deser.required_tests.len(), contract.required_tests.len());
    assert_eq!(deser.rollback_plan, contract.rollback_plan);
    assert_eq!(deser.definition_of_done.len(), contract.definition_of_done.len());
}

// ---------------------------------------------------------------------------
// 2. validate_contract() accepts a complete, well-formed contract
// ---------------------------------------------------------------------------

#[test]
fn validate_accepts_valid_contract() {
    let contract = valid_contract();
    assert!(validate_contract(&contract).is_ok());
}

// ---------------------------------------------------------------------------
// 3. Missing/empty goal → validation failure
// ---------------------------------------------------------------------------

#[test]
fn validate_rejects_empty_goal() {
    let mut contract = valid_contract();
    contract.goal = String::new();
    let errs = validate_contract(&contract).unwrap_err();
    assert!(errs.iter().any(|e| e.field == "goal"));
}

#[test]
fn validate_rejects_whitespace_only_goal() {
    let mut contract = valid_contract();
    contract.goal = "   ".into();
    let errs = validate_contract(&contract).unwrap_err();
    assert!(errs.iter().any(|e| e.field == "goal"));
}

// ---------------------------------------------------------------------------
// 4. Empty acceptance_criteria → validation failure
// ---------------------------------------------------------------------------

#[test]
fn validate_rejects_empty_acceptance_criteria() {
    let mut contract = valid_contract();
    contract.acceptance_criteria.clear();
    let errs = validate_contract(&contract).unwrap_err();
    assert!(errs.iter().any(|e| e.field == "acceptance_criteria"));
}

// ---------------------------------------------------------------------------
// 5. Empty definition_of_done → validation failure
// ---------------------------------------------------------------------------

#[test]
fn validate_rejects_empty_definition_of_done() {
    let mut contract = valid_contract();
    contract.definition_of_done.clear();
    let errs = validate_contract(&contract).unwrap_err();
    assert!(errs.iter().any(|e| e.field == "definition_of_done"));
}

// ---------------------------------------------------------------------------
// 6. Missing task_id (empty string) → validation failure
// ---------------------------------------------------------------------------

#[test]
fn validate_rejects_empty_task_id() {
    let mut contract = valid_contract();
    contract.task_id = String::new();
    let errs = validate_contract(&contract).unwrap_err();
    assert!(errs.iter().any(|e| e.field == "task_id"));
}

#[test]
fn validate_rejects_whitespace_only_task_id() {
    let mut contract = valid_contract();
    contract.task_id = "  \t ".into();
    let errs = validate_contract(&contract).unwrap_err();
    assert!(errs.iter().any(|e| e.field == "task_id"));
}

// ---------------------------------------------------------------------------
// 7. AcceptanceCriterion with empty id or description → validation failure
// ---------------------------------------------------------------------------

#[test]
fn validate_rejects_ac_empty_id() {
    let mut contract = valid_contract();
    contract.acceptance_criteria[0].id = String::new();
    let errs = validate_contract(&contract).unwrap_err();
    assert!(errs.iter().any(|e| e.field.contains("acceptance_criteria[0].id")));
}

#[test]
fn validate_rejects_ac_empty_description() {
    let mut contract = valid_contract();
    contract.acceptance_criteria[0].description = String::new();
    let errs = validate_contract(&contract).unwrap_err();
    assert!(errs.iter().any(|e| e.field.contains("acceptance_criteria[0].description")));
}

// ---------------------------------------------------------------------------
// 8. WiringType variants serialize/deserialize correctly
// ---------------------------------------------------------------------------

#[test]
fn wiring_type_serde_all_variants() {
    let variants = vec![
        WiringType::ModDeclaration,
        WiringType::UseImport,
        WiringType::RouteRegistration,
        WiringType::ConfigLoading,
        WiringType::SerdeDerive,
        WiringType::Other("CustomWiring".into()),
    ];
    for variant in &variants {
        let json = serde_json::to_string(variant).expect("serialize wiring type");
        let deser: WiringType = serde_json::from_str(&json).expect("deserialize wiring type");
        assert_eq!(&deser, variant);
    }
}

// ---------------------------------------------------------------------------
// 9. TestType variants serialize/deserialize correctly
// ---------------------------------------------------------------------------

#[test]
fn test_type_serde_all_variants() {
    let variants = vec![TestType::Unit, TestType::Integration, TestType::E2E, TestType::Property];
    for variant in &variants {
        let json = serde_json::to_string(variant).expect("serialize test type");
        let deser: TestType = serde_json::from_str(&json).expect("deserialize test type");
        assert_eq!(&deser, variant);
    }
}

// ---------------------------------------------------------------------------
// 10. generate_alternatives produces 2-3 alternatives for vague tasks
// ---------------------------------------------------------------------------

#[test]
fn generate_alternatives_for_vague_task() {
    let alts = generate_alternatives("improve performance");
    assert!(
        alts.len() >= 2 && alts.len() <= 3,
        "Expected 2-3 alternatives, got {}",
        alts.len()
    );
    for alt in &alts {
        assert!(alt.goal.contains("improve performance"));
        assert!(!alt.acceptance_criteria.is_empty());
        assert!(!alt.definition_of_done.is_empty());
        // Each alternative should be independently valid
        assert!(validate_contract(alt).is_ok(), "Alternative should be valid");
    }
}

#[test]
fn generate_alternatives_have_unique_ids() {
    let alts = generate_alternatives("refactor auth");
    let ids: Vec<&str> = alts.iter().map(|a| a.task_id.as_str()).collect();
    let unique: std::collections::HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len(), "Alternative IDs must be unique");
}

// ---------------------------------------------------------------------------
// 11. Contract persistence: write to file, read back, fields match
// ---------------------------------------------------------------------------

#[test]
fn contract_persistence_roundtrip() {
    let dir = TempDir::new().expect("create temp dir");
    let contract = valid_contract();

    save_contract(&contract, dir.path()).expect("save");
    let loaded = load_contract(dir.path()).expect("load");

    assert_eq!(loaded.task_id, contract.task_id);
    assert_eq!(loaded.goal, contract.goal);
    assert_eq!(loaded.non_goals, contract.non_goals);
    assert_eq!(loaded.acceptance_criteria.len(), contract.acceptance_criteria.len());
    assert_eq!(loaded.acceptance_criteria[0].id, contract.acceptance_criteria[0].id);
    assert_eq!(loaded.affected_files, contract.affected_files);
    assert_eq!(loaded.rollback_plan, contract.rollback_plan);
    assert_eq!(loaded.definition_of_done, contract.definition_of_done);
}

// ---------------------------------------------------------------------------
// 12. ContractValidatedGate: passes for valid, fails for invalid
// ---------------------------------------------------------------------------

#[test]
fn gate_passes_for_valid_contract() {
    let contract = valid_contract();
    let result = ContractValidatedGate::check(&contract);
    assert!(result.passed, "Gate should pass for valid contract");
    assert!(result.errors.is_empty());
}

#[test]
fn gate_fails_for_invalid_contract() {
    let mut contract = valid_contract();
    contract.goal = String::new();
    contract.task_id = String::new();
    let result = ContractValidatedGate::check(&contract);
    assert!(!result.passed, "Gate should fail for invalid contract");
    assert!(result.errors.len() >= 2, "Should report multiple errors");
}

// ---------------------------------------------------------------------------
// 13. contract-agent replaces task-analyzer as AGENTS[0]
// ---------------------------------------------------------------------------

#[test]
fn contract_agent_is_first_agent() {
    assert_eq!(
        AGENTS[0].key, "contract-agent",
        "AGENTS[0] must be contract-agent (replaces task-analyzer)"
    );
}

// ---------------------------------------------------------------------------
// 14. Multiple validation errors reported at once
// ---------------------------------------------------------------------------

#[test]
fn validate_reports_all_errors_at_once() {
    let contract = TaskContract {
        task_id: String::new(),
        goal: String::new(),
        non_goals: vec![],
        acceptance_criteria: vec![],
        affected_files: vec![],
        required_wiring: vec![],
        required_tests: vec![],
        rollback_plan: String::new(),
        definition_of_done: vec![],
    };
    let errs = validate_contract(&contract).unwrap_err();
    // Should have at least: task_id, goal, acceptance_criteria, definition_of_done
    assert!(errs.len() >= 4, "Should report at least 4 errors, got {}", errs.len());
}
