//! A write branch's declared target set, stamped for the tool guard
//! (Issue-64).
//!
//! The set the ownership gates judge the branch's changes against is the
//! coordinator plan's `target_files` — the item's declaration AFTER
//! `worktree_wave_prepare::widen_to_obligations` added the baseline's
//! obligation files — plus the plan's directory scopes. That widened set,
//! not the raw declaration, is what the guard must refuse writes outside of:
//! an obligation file is one the coder was told to change.
//!
//! Stamped on the branch input under
//! [`crate::agent_dispatch_port::DECLARED_TARGETS_INPUT_KEY`], the host
//! dispatch reads it back and scopes it for
//! `archon_tools::workflow_read_guard`, which refuses a Write/Edit/patch — or
//! a shell write naming its file — at a worktree path outside the set before
//! anything changes. A top-level key, like the forbidden-path stamp: host
//! built, never rendered to the agent, and in
//! `reuse_identity::VOLATILE_INPUT_KEYS` so it never moves the reuse hash.

use archon_write_plan::WritePlan;

use crate::agent_dispatch_port::DECLARED_TARGETS_INPUT_KEY;

/// The repo-relative entries the guard is handed: every target file, and
/// every directory scope with a trailing `/`. Sorted and deduplicated.
pub(super) fn entries(plan: &WritePlan) -> Vec<String> {
    let mut entries: Vec<String> = plan
        .target_files
        .iter()
        .map(|path| path.as_str().to_string())
        .chain(
            plan.target_dir_scopes
                .iter()
                .map(|scope| format!("{}/", scope.as_str().trim_end_matches('/'))),
        )
        .filter(|entry| !entry.is_empty() && entry != "/")
        .collect();
    entries.sort();
    entries.dedup();
    entries
}

/// Stamp the set onto the branch input. Absent when the plan declares
/// nothing: the guard is then inert for the call.
pub(super) fn stamp(input: &mut serde_json::Value, plan: &WritePlan) {
    let entries = entries(plan);
    if entries.is_empty() {
        return;
    }
    if let Some(object) = input.as_object_mut() {
        object.insert(
            DECLARED_TARGETS_INPUT_KEY.to_string(),
            serde_json::json!(entries),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_dispatch_port::declared_targets;

    fn plan(files: &[&str], scopes: &[&str]) -> WritePlan {
        let root = std::path::Path::new("/repo");
        WritePlan {
            run_id: "run".into(),
            stage_id: "stage".into(),
            item_id: "item".into(),
            canonical_root: root.to_path_buf(),
            isolated_root: root.join("iso"),
            target_files: files
                .iter()
                .map(|f| archon_write_plan::normalize_target(f, root).unwrap())
                .collect(),
            target_dir_scopes: scopes
                .iter()
                .map(|f| archon_write_plan::normalize_target(f, root).unwrap())
                .collect(),
            target_files_source: archon_write_plan::TargetFilesSource::Item,
            read_context_files: Vec::new(),
            verify_inputs: Vec::new(),
            baseline_id: "git:HEAD".into(),
            workspace_boundary_required: true,
            resource_keys: Default::default(),
        }
    }

    #[test]
    fn the_stamp_carries_files_and_directory_scopes_and_reads_back_through_the_port() {
        let mut input = serde_json::json!({ "item": { "id": "x" } });
        stamp(
            &mut input,
            &plan(
                &["crates/engine/src/b.rs", "crates/engine/src/a.rs"],
                &["artifacts/runs"],
            ),
        );
        assert_eq!(
            declared_targets(&input),
            vec![
                "artifacts/runs/".to_string(),
                "crates/engine/src/a.rs".to_string(),
                "crates/engine/src/b.rs".to_string(),
            ]
        );
        // The item the agent is rendered is untouched.
        assert_eq!(input["item"], serde_json::json!({ "id": "x" }));
        let mut empty = serde_json::json!({ "item": {} });
        stamp(&mut empty, &plan(&[], &[]));
        assert!(declared_targets(&empty).is_empty());
        assert!(empty.get(DECLARED_TARGETS_INPUT_KEY).is_none());
    }
}
