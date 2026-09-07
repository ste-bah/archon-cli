use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;

fn universe() -> WorkflowV2TaskUniverse {
    let task = |id: &str, deps: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        dependency_ids: deps.iter().map(|d| d.to_string()).collect(),
        ..Default::default()
    };
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task("TASK-001", &[]),
            task("TASK-002", &["TASK-001"]),
            task("TASK-003", &["TASK-001", "TASK-002", "EXTERNAL-9"]),
        ],
    }
}

fn outcome(item: &str, task: &str, status: WorkflowV2Status) -> WorkflowV2BranchOutcome {
    WorkflowV2BranchOutcome {
        item_id: item.into(),
        role: "coder".into(),
        status,
        result: Some(WorkflowV2Result {
            status,
            data: serde_json::json!({"canonical_task_ids": [task]}),
            ..WorkflowV2Result::default()
        }),
        error: None,
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: Vec::new(),
    }
}

fn branch(item: &str, task: &str) -> (String, serde_json::Value) {
    (
        item.into(),
        serde_json::json!({"item": {"item_id": item, "canonical_task_ids": [task]}}),
    )
}

#[test]
fn branch_with_unmet_dependency_is_reported_and_met_one_is_not() {
    let u = universe();
    let branches = vec![branch("i-2", "TASK-002"), branch("i-3", "TASK-003")];
    let none = unmet_dependencies(&branches, Some(&u), &landed_task_ids(&[]));
    assert_eq!(none["i-2"], vec!["TASK-001".to_string()]);
    assert_eq!(
        none["i-3"],
        vec!["TASK-001".to_string(), "TASK-002".to_string()],
        "unknown EXTERNAL-9 never blocks"
    );
    let after_one = landed_task_ids(&[
        outcome("i-1", "TASK-001", WorkflowV2Status::Noop),
        outcome("i-x", "TASK-002", WorkflowV2Status::NeedsReview),
    ]);
    let some = unmet_dependencies(&branches, Some(&u), &after_one);
    assert!(!some.contains_key("i-2"), "noop counts as landed");
    assert_eq!(
        some["i-3"],
        vec!["TASK-002".to_string()],
        "needs_review does not count as landed"
    );
    assert!(
        unmet_dependencies(&branches, None, &[]).is_empty(),
        "no universe, no gate"
    );
}

#[test]
fn blocked_result_is_typed_for_the_script() {
    let (item, input) = branch("i-3", "TASK-003");
    let result =
        blocked_on_dependency_result(&item, &input, Some(&universe()), &["TASK-002".to_string()]);
    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(result.residual_gaps[0].id, "blocked_on_dependency_i-3");
    assert_eq!(result.data["blocked_on_dependency"][0], "TASK-002");
    assert_eq!(result.data["canonical_task_ids"][0], "TASK-003");
    assert_eq!(result.data["item_id"], "i-3");
}
