//! Stamps taken from the authoritative task universe: the tools an item's
//! tasks require and the repository deliverables their contracts declare.
//! Moved out of `write/mod.rs` unchanged. Both write into `input.item` after
//! the reuse identity is stamped: `required_tools` is also stripped from the
//! projection (`reuse_identity::VOLATILE_ITEM_KEYS`), and the `target_files`
//! rewrite is taken after the authored value has been hashed.
use super::*;

/// Admit each item's declared repository deliverables to its writable targets.
///
/// Host-parsed contracts only, and only for items that already own repository
/// code — an artifact-only item is served by `add_contract_artifact_paths` and
/// must not acquire code writes here. Paths under an artifact root stay
/// artifacts.
pub(super) fn stamp_contract_code_targets(
    branches: &mut [crate::WorkflowV2FanoutItem],
    task_universe: Option<&crate::task_universe::WorkflowV2TaskUniverse>,
    v2_store: &WorkflowV2ResultStore,
) {
    let Some(universe) = task_universe else {
        return;
    };
    let artifact_roots =
        crate::v2::project_artifacts::project_artifact_context_from_v2_root(v2_store.root())
            .artifact_roots;
    for branch in branches.iter_mut() {
        let Some(item) = branch.input.get("item") else {
            continue;
        };
        let added = crate::v2::contract_code_targets::contract_code_targets_for_item(
            universe,
            item,
            &artifact_roots,
        );
        if added.is_empty() {
            continue;
        }
        let mut targets = branch.call.options.target_files.clone();
        for path in &added {
            if !targets.contains(path) {
                targets.push(path.clone());
            }
        }
        branch.call.options.target_files = targets.clone();
        if let Some(object) = branch
            .input
            .get_mut("item")
            .and_then(serde_json::Value::as_object_mut)
        {
            object.insert("target_files".to_string(), serde_json::json!(targets));
        }
    }
}

/// Stamp each branch item's `required_tools` from the AUTHORITATIVE task
/// universe, matched by the item's canonical task ids. Runs for every write
/// branch regardless of whether a source graph was built, so tool binding
/// works for authored (v3) and generated (v2) call ids alike. Agent-authored
/// tool declarations were already stripped at the shared builder, so this is
/// the only writer of the field.
pub(super) fn stamp_required_tools_from_universe(
    branches: &mut [crate::WorkflowV2FanoutItem],
    task_universe: Option<&crate::task_universe::WorkflowV2TaskUniverse>,
) {
    let Some(universe) = task_universe else {
        return;
    };
    for branch in branches {
        let Some(item) = branch
            .input
            .get_mut("item")
            .and_then(serde_json::Value::as_object_mut)
        else {
            continue;
        };
        let claimed: Vec<String> = item
            .get("canonical_task_ids")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect();
        if claimed.is_empty() {
            continue;
        }
        let mut tools: std::collections::BTreeSet<String> = Default::default();
        for task in &universe.tasks {
            if claimed.iter().any(|id| id == &task.canonical_task_id) {
                tools.extend(task.required_tools.iter().cloned());
            }
        }
        if !tools.is_empty() {
            item.insert(
                "required_tools".to_string(),
                serde_json::json!(tools.into_iter().collect::<Vec<_>>()),
            );
        }
    }
}
