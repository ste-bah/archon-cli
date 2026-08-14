use super::*;
use crate::task_universe::{
    WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};

fn universe(task_id: &str, paths: &[&str]) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: task_id.to_string(),
            deliverable_contracts: paths
                .iter()
                .map(|p| WorkflowV2DeliverableContract {
                    kind: "create".to_string(),
                    artifact_path: (*p).to_string(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }],
    }
}

fn item(tasks: &[&str], targets: &[&str]) -> Value {
    serde_json::json!({ "canonical_task_ids": tasks, "target_files": targets })
}

const ARTIFACTS: [&str; 1] = [".archon"];

fn roots() -> Vec<String> {
    ARTIFACTS.iter().map(|s| s.to_string()).collect()
}

/// The live failure: TDL-080 declared coverage.rs and coverage_tests.rs as
/// contracts, not targets. The agent produced both and the write layer dropped
/// them, then the completion check failed the branch for their absence.
#[test]
fn a_declared_source_deliverable_becomes_a_target() {
    let u = universe(
        "TASK-TDL-080",
        &[
            "crates/archon-trading/src/data_store/coverage.rs",
            "crates/archon-trading/src/data_store/coverage_tests.rs",
        ],
    );
    let it = item(&["TASK-TDL-080"], &["src/command/trading.rs"]);
    assert_eq!(
        contract_code_targets_for_item(&u, &it, &roots()),
        vec![
            "crates/archon-trading/src/data_store/coverage.rs".to_string(),
            "crates/archon-trading/src/data_store/coverage_tests.rs".to_string(),
        ]
    );
}

/// A contract under an artifact root is a project artifact and must not become
/// a repository write target.
#[test]
fn a_project_artifact_is_not_admitted_as_code() {
    let u = universe(
        "TASK-TDL-080",
        &[".archon/trading-lab/data/coverage/latest.json"],
    );
    let it = item(&["TASK-TDL-080"], &["src/command/trading.rs"]);
    assert!(contract_code_targets_for_item(&u, &it, &roots()).is_empty());
}

/// An artifact-only item is served by the artifact path and gains nothing here
/// — admitting code paths would hand it writes it was never scoped for.
#[test]
fn an_artifact_only_item_gains_nothing() {
    let u = universe("TASK-TDL-001", &["crates/thing/src/lib.rs"]);
    let it = item(&["TASK-TDL-001"], &[]);
    assert!(contract_code_targets_for_item(&u, &it, &roots()).is_empty());
}

/// Already-declared targets are not duplicated.
#[test]
fn an_existing_target_is_not_repeated() {
    let u = universe("TASK-TDL-080", &["src/command/trading.rs"]);
    let it = item(&["TASK-TDL-080"], &["src/command/trading.rs"]);
    assert!(contract_code_targets_for_item(&u, &it, &roots()).is_empty());
}

/// Templates, globs, absolute paths, traversal and directories name no single
/// repository file and are refused.
#[test]
fn unusable_shapes_are_refused() {
    let u = universe(
        "TASK-TDL-080",
        &[
            "${PROJECT_ROOT}/out.rs",
            "src/**/*.rs",
            "/etc/passwd",
            "../outside.rs",
            "crates/thing/src/",
            "   ",
        ],
    );
    let it = item(&["TASK-TDL-080"], &["src/command/trading.rs"]);
    assert!(contract_code_targets_for_item(&u, &it, &roots()).is_empty());
}

/// Another task's contracts are never admitted.
#[test]
fn only_the_items_own_tasks_contribute() {
    let u = universe("TASK-TDL-080", &["crates/thing/src/coverage.rs"]);
    let it = item(&["TASK-TDL-090"], &["src/command/trading.rs"]);
    assert!(contract_code_targets_for_item(&u, &it, &roots()).is_empty());
}
