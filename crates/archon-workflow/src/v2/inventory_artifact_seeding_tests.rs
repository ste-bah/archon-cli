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
                .map(|path| WorkflowV2DeliverableContract {
                    kind: "create".to_string(),
                    artifact_path: (*path).to_string(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }],
    }
}

fn requirements(inventory: &Value) -> Vec<String> {
    inventory["items"][0]["artifact_requirements"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The live failure: TASK-TDL-001 declares its report as a deliverable
/// contract and no `artifact_requirements:` key, so the item arrived empty and
/// six repair rounds could not reconstruct the path a reducer was never told
/// to look for.
#[test]
fn a_declared_deliverable_becomes_a_concrete_requirement() {
    let inventory = serde_json::json!({
        "items": [{
            "item_id": "impl-TASK-TDL-001-data-lake-gap-audit",
            "canonical_task_ids": ["TASK-TDL-001"],
            "target_files": [],
        }]
    });
    let seeded = seed_artifact_requirements(
        &universe("TASK-TDL-001", &["docs/trading/data-lake-gap-audit.md"]),
        &inventory,
    );
    assert_eq!(
        requirements(&seeded),
        vec!["docs/trading/data-lake-gap-audit.md"]
    );
}

/// A contract path the item also claims as a repository target is code, not a
/// project artifact. Seeding it would declare source files as artifacts and
/// route them past write-ownership.
#[test]
fn a_path_the_item_owns_as_a_repository_target_is_not_an_artifact() {
    let inventory = serde_json::json!({
        "items": [{
            "item_id": "impl-TASK-TDL-040",
            "canonical_task_ids": ["TASK-TDL-040"],
            "target_files": ["crates/archon-trading/src/data_lake/tradingview_mcp.rs"],
        }]
    });
    let seeded = seed_artifact_requirements(
        &universe(
            "TASK-TDL-040",
            &["crates/archon-trading/src/data_lake/tradingview_mcp.rs"],
        ),
        &inventory,
    );
    assert!(
        requirements(&seeded).is_empty(),
        "a declared repository target must not become an artifact requirement"
    );
}

/// A mixed task keeps only what it does not build as code.
#[test]
fn mixed_contracts_keep_only_the_artifacts() {
    let inventory = serde_json::json!({
        "items": [{
            "canonical_task_ids": ["TASK-TDL-080"],
            "target_files": ["crates/thing/src/coverage.rs"],
        }]
    });
    let seeded = seed_artifact_requirements(
        &universe(
            "TASK-TDL-080",
            &["crates/thing/src/coverage.rs", "docs/coverage-matrix.md"],
        ),
        &inventory,
    );
    assert_eq!(requirements(&seeded), vec!["docs/coverage-matrix.md"]);
}

/// An item that already declares requirements is left exactly as it is.
#[test]
fn an_item_that_declares_requirements_is_untouched() {
    let inventory = serde_json::json!({
        "items": [{
            "canonical_task_ids": ["TASK-TDL-001"],
            "target_files": [],
            "artifact_requirements": ["reports/authored-by-the-reducer.md"],
        }]
    });
    let seeded = seed_artifact_requirements(
        &universe("TASK-TDL-001", &["docs/trading/data-lake-gap-audit.md"]),
        &inventory,
    );
    assert_eq!(
        requirements(&seeded),
        vec!["reports/authored-by-the-reducer.md"]
    );
}

/// A templated path is not concrete; stamping it would assert a literal
/// `${VAR}` deliverable, which is what `stamp_artifact_presence` refuses to
/// stat for the same reason.
#[test]
fn templated_paths_are_never_seeded() {
    let inventory = serde_json::json!({
        "items": [{ "canonical_task_ids": ["TASK-TDL-001"], "target_files": [] }]
    });
    let seeded = seed_artifact_requirements(
        &universe("TASK-TDL-001", &["${PROJECT_ROOT}/out.json"]),
        &inventory,
    );
    assert!(requirements(&seeded).is_empty());
}

/// An item speaking for a task the universe does not carry gains nothing.
#[test]
fn an_unknown_task_seeds_nothing() {
    let inventory = serde_json::json!({
        "items": [{ "canonical_task_ids": ["TASK-TDL-999"], "target_files": [] }]
    });
    let seeded =
        seed_artifact_requirements(&universe("TASK-TDL-001", &["docs/report.md"]), &inventory);
    assert!(requirements(&seeded).is_empty());
}
