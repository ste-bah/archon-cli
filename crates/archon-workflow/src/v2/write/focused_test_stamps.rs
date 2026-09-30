//! Batch O (I1): a branch runs the focused tests its tasks DECLARE, never
//! the authored script's copy of them.
//!
//! The authoring model hand-copies each task's `## Focused Tests` into the
//! script's `focusedTests`, and nothing compared the copy with the task
//! file: a dropped, altered or invented command changed what the base-commit
//! baseline ran, what the write branch was widened to, and what the
//! verifier judged the task by. At dispatch the host now compares each
//! branch item's list with the union of its own tasks' declared commands
//! (`WorkflowV2TaskUniverseTask::focused_tests`) and, on any difference,
//! replaces it with the declared set -- recording the authored list beside
//! it so the substitution is visible.
//!
//! Left alone: an item with no task, or whose tasks declare no command; a
//! host-planned residual round or an escalated round (their commands are the
//! host's plan for exactly those files); and a read-only goal verifier that
//! carries no command at all (it states `verification_requirements`
//! instead, and the host demotes one that runs nothing).

use crate::task_universe::WorkflowV2TaskUniverse;

/// The item key the authored copy is kept under when it is replaced.
pub const AUTHORED_FOCUSED_TESTS_KEY: &str = "authored_focused_verification";

const FOCUSED_KEYS: [&str; 3] = ["focused_verification", "focused_tests", "focusedTests"];

/// Replace every branch item's focused tests that differ from its tasks'
/// declared commands with the declared commands.
pub fn stamp_declared_focused_tests(
    branches: &mut [crate::WorkflowV2FanoutItem],
    universe: Option<&WorkflowV2TaskUniverse>,
) {
    let Some(universe) = universe else {
        return;
    };
    for branch in branches.iter_mut() {
        if let Some(item) = branch
            .input
            .get_mut("item")
            .and_then(serde_json::Value::as_object_mut)
        {
            reconcile(item, universe);
        }
    }
}

fn strings(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

fn reconcile(
    item: &mut serde_json::Map<String, serde_json::Value>,
    universe: &WorkflowV2TaskUniverse,
) {
    let host_planned = item.contains_key(crate::v2::script::residual_plan::RESIDUAL_ITEM_PATHS_KEY)
        || item.contains_key("escalation_owner_task_ids");
    if host_planned {
        return;
    }
    let task_ids = strings(item.get("canonical_task_ids"));
    let mut declared: Vec<String> = Vec::new();
    for task in universe
        .tasks
        .iter()
        .filter(|task| task_ids.contains(&task.canonical_task_id))
    {
        for command in &task.focused_tests {
            let command = command.trim().to_string();
            if !command.is_empty() && !declared.contains(&command) {
                declared.push(command);
            }
        }
    }
    if declared.is_empty() {
        return;
    }
    let authored = FOCUSED_KEYS
        .iter()
        .find_map(|key| item.get(*key).filter(|value| value.is_array()))
        .map(|value| strings(Some(value)))
        .unwrap_or_default();
    let writes =
        item.get("work_type").and_then(serde_json::Value::as_str) == Some("implementation");
    let goal = !strings(item.get("verification_requirements")).is_empty();
    if !writes && (authored.is_empty() || goal) {
        // A goal verifier: it proves its requirements with commands of its
        // own choosing, and the host demotes one that runs none.
        return;
    }
    let mut same = authored.clone();
    same.sort();
    same.dedup();
    let mut want = declared.clone();
    want.sort();
    if same == want {
        return;
    }
    item.insert(
        AUTHORED_FOCUSED_TESTS_KEY.to_string(),
        serde_json::json!(authored),
    );
    for key in FOCUSED_KEYS {
        item.remove(key);
    }
    item.insert(
        "focused_verification".to_string(),
        serde_json::json!(declared),
    );
}

#[cfg(test)]
#[path = "focused_test_stamps_tests.rs"]
mod tests;
