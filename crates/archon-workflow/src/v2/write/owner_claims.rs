//! Issue-121: a write branch is never granted another task's file.
//!
//! The ownership grant (`worktree_scope_grant`) keeps an undeclared change
//! inside the plan's scope roots that no OTHER item of the wave claims. A
//! single-item wave contests nothing, so every task outside the wave was
//! invisible to it. On a live run a residual round served one task, changed
//! a store module another task declares in its Files Expected to Change,
//! was granted it, and landed a regression that left that task's declared
//! must-pass tests red.
//!
//! So every task of the universe that is NOT one of the branch's canonical
//! tasks claims its declared paths, exactly as a sibling item of the wave
//! claims its targets: the grant contests such a path and refuses it by the
//! existing rule for a claimed path, the tool guard reads the same claims
//! from the `_grantable_scope` stamp (`declared_targets::stamp_grantable`)
//! and refuses the write when it is attempted, and the adapter's claim check
//! reads them from the call. One list, so guard and landing cannot disagree.
//!
//! A path one of the branch's own tasks declares is the branch's: it is
//! left out of every other task's claim. Declared paths are read with the
//! host's one ownership reader (`path_ownership::declared_paths_of`, the
//! files expected to change, shared-append targets and deliverable contract
//! paths) in the repository-relative form; one no reader can place in the
//! repository is not a path a worktree can change. Without a task universe
//! nothing is added, which is the pre-existing behaviour.

use std::collections::BTreeSet;
use std::path::Path;

use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::verification::path_ownership::{
    DeclaredPathForm, declared_covers, declared_path_form, declared_paths_of,
};
use crate::v2::write_scope_extension::WaveClaim;

/// Prefix of an owner claim's holder: never a write item's id, which the
/// wave planner derives from call ids.
pub(crate) const OWNER_CLAIM_PREFIX: &str = "task-owner:";

/// One claim per universe task outside `own_task_ids`, holding the paths it
/// declares that none of `own_task_ids` declares; tasks declaring nothing
/// else are omitted.
pub(crate) fn owner_claims(
    universe: Option<&WorkflowV2TaskUniverse>,
    own_task_ids: &[String],
    repository_root: &Path,
) -> Vec<WaveClaim> {
    let Some(universe) = universe else {
        return Vec::new();
    };
    let own = |id: &str| own_task_ids.iter().any(|own| own == id);
    let own_paths: BTreeSet<String> = universe
        .tasks
        .iter()
        .filter(|task| own(&task.canonical_task_id))
        .flat_map(|task| repo_paths(task, repository_root))
        .collect();
    universe
        .tasks
        .iter()
        .filter(|task| !own(&task.canonical_task_id))
        .filter_map(|task| {
            let owned: Vec<String> = repo_paths(task, repository_root)
                .into_iter()
                .filter(|path| !own_paths.iter().any(|mine| declared_covers(mine, path)))
                .collect();
            (!owned.is_empty()).then(|| {
                WaveClaim::new(
                    format!("{OWNER_CLAIM_PREFIX}{}", task.canonical_task_id),
                    owned,
                )
            })
        })
        .collect()
}

/// Every declared path of `task`, repository-relative, in the form the
/// grant's claims compare (`crate::v2::normalize_target_for_repository`).
fn repo_paths(
    task: &crate::task_universe::WorkflowV2TaskUniverseTask,
    repository_root: &Path,
) -> BTreeSet<String> {
    let root = repository_root.display().to_string();
    declared_paths_of(task)
        .iter()
        .filter_map(|entry| match declared_path_form(entry, repository_root) {
            DeclaredPathForm::Repo(path) => Some(path.trim_end_matches("/**").to_string()),
            _ => None,
        })
        .filter_map(|path| {
            crate::v2::normalize_target_for_repository(&task.canonical_task_id, &path, Some(&root))
                .ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_universe::WorkflowV2TaskUniverseTask;

    fn task(id: &str, files: &[&str]) -> WorkflowV2TaskUniverseTask {
        WorkflowV2TaskUniverseTask {
            canonical_task_id: id.into(),
            files_expected_to_change: files.iter().map(|f| (*f).to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn every_other_task_claims_what_it_declares_and_a_shared_path_stays_the_branchs() {
        let universe = WorkflowV2TaskUniverse {
            schema_version: "t".into(),
            source_roots: Vec::new(),
            tasks: vec![
                task(
                    "T-1",
                    &["`crates/a/src/lib.rs` — own", "crates/a/src/shared.rs"],
                ),
                task(
                    "T-2",
                    &[
                        "/repo/crates/a/src/store.rs — exists",
                        "crates/a/src/shared.rs",
                        "crates/a/data/",
                    ],
                ),
                task("T-3", &["crates/a/src/lib.rs"]),
            ],
        };
        let root = Path::new("/repo");
        let claims = owner_claims(Some(&universe), &["T-1".to_string()], root);
        assert_eq!(
            claims,
            vec![WaveClaim::new(
                "task-owner:T-2",
                [
                    "crates/a/data".to_string(),
                    "crates/a/src/store.rs".to_string()
                ]
            )],
            "T-3 declares only T-1's own file; T-2's shared file stays T-1's"
        );
        assert!(owner_claims(None, &["T-1".to_string()], root).is_empty());
        // A branch serving no task holds nothing of anyone's.
        assert_eq!(owner_claims(Some(&universe), &[], root).len(), 3);
    }
}
