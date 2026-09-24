//! Stamps taken from the authoritative task universe: the tools an item's
//! tasks require, the repository deliverables their contracts declare, and
//! the files those tasks declare they expect to change.
//! Moved out of `write/mod.rs` unchanged. They write into `input.item` after
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
        admit_targets(branch, &added);
    }
}

/// Admit each item's own tasks' DECLARED writable scope to its targets.
///
/// The floor under every write branch: the verifier judges a task against the
/// paths its task file declares, so a branch remediating that task must be
/// able to write them. A branch whose call declared fewer of them than the
/// task does could never satisfy the verifier and could never fail either —
/// see [`crate::v2::task_declared_targets`] for the deadlock and for what is
/// refused. Union, never substitution: whatever the call already declared is
/// kept, and only this item's own tasks contribute, so no branch gains a path
/// another task owns.
pub(super) fn stamp_task_declared_targets(
    branches: &mut [crate::WorkflowV2FanoutItem],
    task_universe: Option<&crate::task_universe::WorkflowV2TaskUniverse>,
    v2_store: &WorkflowV2ResultStore,
    target_repository_root: Option<&str>,
) {
    let Some(universe) = task_universe else {
        return;
    };
    let artifact_roots =
        crate::v2::project_artifacts::project_artifact_context_from_v2_root(v2_store.root())
            .artifact_roots;
    let repository_root = target_repository_root
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .map(Path::new);
    for branch in branches.iter_mut() {
        let Some(item) = branch.input.get("item") else {
            continue;
        };
        let added = crate::v2::task_declared_targets::task_declared_code_targets_for_item(
            universe,
            item,
            &artifact_roots,
            repository_root,
        );
        admit_targets(branch, &added);
    }
}

/// Add `added` to one branch's targets, on the call and on its item alike:
/// the call is what the plan is built from, the item is what
/// `target_files_from_item` reads. Silent when nothing is added.
fn admit_targets(branch: &mut crate::WorkflowV2FanoutItem, added: &[String]) {
    if added.is_empty() {
        return;
    }
    let mut targets = branch.call.options.target_files.clone();
    for path in added {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::{WorkflowV2HostMethod, WorkflowV2HostOptions};

    fn branch(targets: &[&str]) -> crate::WorkflowV2FanoutItem {
        let call = WorkflowV2HostCall {
            id: "wave-1-item".to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: WorkflowV2HostOptions {
                target_files: targets.iter().map(|t| (*t).to_string()).collect(),
                target_files_from_item: true,
                ..WorkflowV2HostOptions::default()
            },
        };
        crate::WorkflowV2FanoutItem::read_only(
            "wave-1-item",
            "coder",
            call,
            serde_json::json!({ "item": { "item_id": "item", "target_files": targets } }),
        )
    }

    /// The floor is a union: the call keeps everything it declared, on the
    /// call and on the item alike, and the added paths follow it. Replacing
    /// either list would drop a target the branch legitimately owns.
    #[test]
    fn admitted_targets_are_added_to_what_the_branch_already_declared() {
        let mut branch = branch(&["src/owner.rs", "src/owner/split.rs"]);
        admit_targets(&mut branch, &["src/helper.rs".to_string()]);
        assert_eq!(
            branch.call.options.target_files,
            vec![
                "src/owner.rs".to_string(),
                "src/owner/split.rs".to_string(),
                "src/helper.rs".to_string(),
            ]
        );
        assert_eq!(
            branch.input["item"]["target_files"],
            serde_json::json!(["src/owner.rs", "src/owner/split.rs", "src/helper.rs"])
        );
    }

    /// A path already declared is not repeated, and nothing to add leaves the
    /// branch byte-identical.
    #[test]
    fn nothing_to_add_changes_nothing() {
        let mut branch = branch(&["src/owner.rs"]);
        let before = branch.clone();
        admit_targets(&mut branch, &[]);
        assert_eq!(
            branch.call.options.target_files,
            before.call.options.target_files
        );
        assert_eq!(branch.input, before.input);
        admit_targets(&mut branch, &["src/owner.rs".to_string()]);
        assert_eq!(
            branch.call.options.target_files,
            vec!["src/owner.rs".to_string()]
        );
    }
}
