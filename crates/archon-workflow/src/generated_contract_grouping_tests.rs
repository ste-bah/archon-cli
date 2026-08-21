//! One item, one deliverable contract.
//!
//! Built from the inventory that actually shipped on wf-3d7efd28, not from an
//! invented example: `TASK-TDL-040-050-060-070-providers` claimed four
//! canonical tasks and `TASK-TDL-010-010-020-030-base` claimed three. Three of
//! that first group never appeared in any implementation wave, because they had
//! no item to be dispatched as, and the group closed on one result.

use super::*;
use crate::task_universe::{
    WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};

fn contracted(id: &str) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.to_string(),
        source_path: format!("/tmp/{id}.md"),
        deliverable_contracts: vec![WorkflowV2DeliverableContract {
            kind: "dataset".to_string(),
            artifact_path: format!("data/{id}.json"),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn uncontracted(id: &str) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.to_string(),
        source_path: format!("/tmp/{id}.md"),
        ..Default::default()
    }
}

fn universe_of(tasks: Vec<WorkflowV2TaskUniverseTask>) -> ContractTaskUniverse {
    ContractTaskUniverse::from_authoritative(Some(&WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["/tmp".to_string()],
        tasks,
    }))
}

fn item(id: &str, task_ids: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "item_id": id,
        "work_type": "implementation",
        "canonical_task_ids": task_ids,
        "target_files": ["crates/x/src/lib.rs"],
        "acceptance_criteria": ["the ingest persists datasets"],
        "focused_verification": ["cargo test -p x"],
        "artifact_requirements": ["data/registry.json"],
    })
}

fn fields(issues: &[GeneratedContractIssue]) -> Vec<String> {
    issues.iter().map(|issue| issue.field.clone()).collect()
}

/// THE live item. Four contracted tasks, one result, three tasks never run.
#[test]
fn the_providers_group_that_never_fired_is_refused() {
    let universe = universe_of(vec![
        contracted("TASK-TDL-040"),
        contracted("TASK-TDL-050"),
        contracted("TASK-TDL-060"),
        contracted("TASK-TDL-070"),
    ]);
    let issues = generated_item_issues(
        &item(
            "TASK-TDL-040-050-060-070-providers",
            &[
                "TASK-TDL-040",
                "TASK-TDL-050",
                "TASK-TDL-060",
                "TASK-TDL-070",
            ],
        ),
        &universe,
        None,
    );
    assert!(
        fields(&issues).contains(&"canonical_task_ids".to_string()),
        "a four-task group must be split, got: {:?}",
        fields(&issues)
    );
}

/// One contracted task per item is the shape this asks for.
#[test]
fn a_single_contracted_task_is_accepted() {
    let universe = universe_of(vec![contracted("TASK-TDL-050")]);
    let issues = generated_item_issues(&item("impl-tdl-050", &["TASK-TDL-050"]), &universe, None);
    assert!(
        !fields(&issues).contains(&"canonical_task_ids".to_string()),
        "got: {:?}",
        fields(&issues)
    );
}

/// Scoped deliberately. Tasks with no contract of their own have no separate
/// proof to lose, so they may still share an item — refusing them too would
/// force splits that buy nothing and cost a turn each.
#[test]
fn uncontracted_tasks_may_still_share_an_item() {
    let universe = universe_of(vec![
        contracted("TASK-TDL-040"),
        uncontracted("TASK-TDL-041"),
        uncontracted("TASK-TDL-042"),
    ]);
    let issues = generated_item_issues(
        &item("group", &["TASK-TDL-040", "TASK-TDL-041", "TASK-TDL-042"]),
        &universe,
        None,
    );
    assert!(
        !fields(&issues).contains(&"canonical_task_ids".to_string()),
        "only ONE contracted task here, got: {:?}",
        fields(&issues)
    );
}

/// A no-op item is not what this rule is for: it has its own bar to clear, and
/// the refuted-no-op and execution rules already police it.
#[test]
fn the_rule_targets_implementation_items() {
    let universe = universe_of(vec![contracted("TASK-TDL-040"), contracted("TASK-TDL-050")]);
    let mut value = item("group", &["TASK-TDL-040", "TASK-TDL-050"]);
    value["work_type"] = serde_json::json!("verified_noop");
    value["noop_proof"] = serde_json::json!("already done");
    value["noop_proof_refs"] = serde_json::json!(["src/x.rs"]);

    let issues = generated_item_issues(&value, &universe, None);
    assert!(
        !fields(&issues).contains(&"canonical_task_ids".to_string()),
        "got: {:?}",
        fields(&issues)
    );
}
