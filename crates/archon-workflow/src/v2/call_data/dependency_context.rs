//! Carry upstream consumed artifacts as read context, never as writable contracts.
use crate::task_universe::WorkflowV2TaskUniverse;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn attach(input: &mut Value, universe: &WorkflowV2TaskUniverse) {
    let claimed = crate::v2::branch_stamping::branch_canonical_task_ids(input);
    if claimed.is_empty() {
        return;
    }
    let tasks = universe
        .tasks
        .iter()
        .map(|task| (task.canonical_task_id.as_str(), task))
        .collect::<BTreeMap<_, _>>();
    let mut pending = claimed.clone();
    let mut visited = BTreeSet::new();
    let mut paths = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id.clone()) {
            continue;
        }
        let Some(task) = tasks.get(id.as_str()) else {
            continue;
        };
        for dependency in &task.dependencies {
            pending.push(dependency.task_id.clone());
            for artifact in &dependency.consumes {
                paths.insert((dependency.task_id.clone(), artifact.artifact_path.clone()));
            }
        }
        pending.extend(task.dependency_ids.iter().cloned());
    }
    if paths.is_empty() {
        return;
    }
    let project = input
        .get("_workflow_project_artifact_policy")
        .and_then(|p| p.get("project_root"))
        .cloned();
    let context = json!({"purpose":"read-only dependency context; inspect these upstream artifacts before implementation; no write authority or completion obligation is granted",
        "project_root":project,"references":paths.into_iter().map(|(producer,path)|json!({"producer_task":producer,"read_path":path})).collect::<Vec<_>>()});
    match input {
        Value::Object(map) => {
            map.insert("dependency_read_context".into(), context);
        }
        _ => *input = json!({"item_input":input,"dependency_read_context":context}),
    }
}
