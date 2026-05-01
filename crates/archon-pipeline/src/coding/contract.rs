//! TaskContract schema, validation, gate, and alternative generation.
//!
//! Implements REQ-IMPROVE-001 (contract-agent) and EC-PIPE-008
//! (alternative generation for vague tasks).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A structured contract describing exactly what a pipeline task will deliver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskContract {
    pub task_id: String,
    pub goal: String,
    pub non_goals: Vec<String>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub affected_files: Vec<String>,
    pub required_wiring: Vec<WiringRequirement>,
    pub required_tests: Vec<TestRequirement>,
    pub rollback_plan: String,
    pub definition_of_done: Vec<String>,
}

/// A single acceptance criterion with a verification method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    pub id: String,
    pub description: String,
    pub verification: String,
}

/// A wiring requirement — how the new code integrates with existing modules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WiringRequirement {
    pub module: String,
    pub entrypoint: String,
    pub wiring_type: WiringType,
}

/// The kind of wiring needed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WiringType {
    ModDeclaration,
    UseImport,
    RouteRegistration,
    ConfigLoading,
    SerdeDerive,
    Other(String),
}

/// A test that must exist before the task is considered done.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRequirement {
    pub test_type: TestType,
    pub description: String,
}

/// The category of test.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TestType {
    Unit,
    Integration,
    E2E,
    Property,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// A single validation error describing which field failed and why.
#[derive(Debug, Clone)]
pub struct ContractValidationError {
    pub field: String,
    pub message: String,
}

impl std::fmt::Display for ContractValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl std::error::Error for ContractValidationError {}

/// Validate a [`TaskContract`], returning all errors found (not just the first).
pub fn validate_contract(
    contract: &TaskContract,
) -> std::result::Result<(), Vec<ContractValidationError>> {
    let mut errors = Vec::new();

    if contract.task_id.trim().is_empty() {
        errors.push(ContractValidationError {
            field: "task_id".into(),
            message: "must not be empty".into(),
        });
    }
    if contract.goal.trim().is_empty() {
        errors.push(ContractValidationError {
            field: "goal".into(),
            message: "must not be empty".into(),
        });
    }
    if contract.acceptance_criteria.is_empty() {
        errors.push(ContractValidationError {
            field: "acceptance_criteria".into(),
            message: "must have at least one entry".into(),
        });
    }
    if contract.definition_of_done.is_empty() {
        errors.push(ContractValidationError {
            field: "definition_of_done".into(),
            message: "must have at least one entry".into(),
        });
    }

    // Validate each acceptance criterion has non-empty id and description.
    for (i, ac) in contract.acceptance_criteria.iter().enumerate() {
        if ac.id.trim().is_empty() {
            errors.push(ContractValidationError {
                field: format!("acceptance_criteria[{}].id", i),
                message: "must not be empty".into(),
            });
        }
        if ac.description.trim().is_empty() {
            errors.push(ContractValidationError {
                field: format!("acceptance_criteria[{}].description", i),
                message: "must not be empty".into(),
            });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// ---------------------------------------------------------------------------
// Gate
// ---------------------------------------------------------------------------

/// Result of a gate check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateResult {
    pub passed: bool,
    pub errors: Vec<String>,
}

/// Gate that verifies a contract is valid before allowing the pipeline to
/// proceed.
pub struct ContractValidatedGate;

impl ContractValidatedGate {
    /// Check whether the given contract passes validation.
    pub fn check(contract: &TaskContract) -> GateResult {
        match validate_contract(contract) {
            Ok(()) => GateResult {
                passed: true,
                errors: vec![],
            },
            Err(errs) => GateResult {
                passed: false,
                errors: errs.iter().map(|e| e.to_string()).collect(),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Alternative generation (EC-PIPE-008)
// ---------------------------------------------------------------------------

/// For vague tasks, generate 2-3 alternative contract scopes.
///
/// The actual LLM call happens at runtime; this function provides the
/// template structure and default alternative generation logic.
pub fn generate_alternatives(vague_goal: &str) -> Vec<TaskContract> {
    let alternatives = [("narrow", "Focused minimal scope"),
        ("moderate", "Balanced scope with key improvements"),
        ("broad", "Comprehensive scope covering all aspects")];

    alternatives
        .iter()
        .enumerate()
        .map(|(i, (scope, desc))| TaskContract {
            task_id: format!("ALT-{}", i + 1),
            goal: format!("{} [{}]: {}", vague_goal, scope, desc),
            non_goals: vec![format!("Out of scope for {} approach", scope)],
            acceptance_criteria: vec![AcceptanceCriterion {
                id: format!("AC-ALT-{}-001", i + 1),
                description: format!("Primary deliverable for {} scope", scope),
                verification: "Manual review".into(),
            }],
            affected_files: vec![],
            required_wiring: vec![],
            required_tests: vec![TestRequirement {
                test_type: TestType::Unit,
                description: format!("Tests for {} scope", scope),
            }],
            rollback_plan: "Revert commit".into(),
            definition_of_done: vec![format!("{} scope completed and verified", scope)],
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

/// Save a contract to `<session_dir>/contract.json`.
pub fn save_contract(contract: &TaskContract, session_dir: &Path) -> Result<()> {
    let path = session_dir.join("contract.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(contract)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Load a contract from `<session_dir>/contract.json`.
pub fn load_contract(session_dir: &Path) -> Result<TaskContract> {
    let path = session_dir.join("contract.json");
    let data = std::fs::read_to_string(&path)?;
    let contract: TaskContract = serde_json::from_str(&data)?;
    Ok(contract)
}
