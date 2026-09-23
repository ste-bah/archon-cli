//! Who declares a repository path, told to the verifier before it judges
//! (Issue-85).
//!
//! A verifier can find a real defect in a file that NO task declares. No
//! branch may write such a file, so withholding acceptance over it cannot
//! produce a fix — remediation returns a no-op and the cycle repeats until
//! the budget is gone. The verifier is the only party that can decline to
//! withhold, because the host must never promote a verdict from prose, so
//! the verifier has to be told which paths are nobody's.
//!
//! It cannot work that out for itself: the task universe never travels to a
//! read-only branch, and no task's declared paths are rendered to it. So the
//! host answers the question here, before dispatch, and stamps the answer.
//!
//! ## Why this lists what IS declared, not what is not
//!
//! The obvious stamp — "the paths no task declares" — cannot be built. The
//! only path that matters is the one the verifier is about to blame, which
//! is not known until it reports; enumerating every other file in the
//! repository instead would be enormous and would still miss that one.
//!
//! So the set is inverted. Every path every task declares is a list bounded
//! by the task universe — tens of entries for a real task set — and it is
//! COMPLETE, which is what makes it decisive: a repository path in neither
//! list is declared by no task. That answers the question for any path,
//! including one nobody could have enumerated in advance, and it is the same
//! shape as the base-commit stamp's other-owner list, which likewise carries
//! the host's conclusion rather than the evidence behind it. The universe
//! itself still never reaches the verifier — only paths and owning task ids.
//!
//! ## What it cannot cover
//!
//! - A declared entry that names a DIRECTORY covers every file beneath it;
//!   membership is not string equality and the prompt says so.
//! - It reflects what tasks DECLARE. A path a task should have declared and
//!   did not reads as nobody's, which is the defect this exists to surface,
//!   not one it can correct.
//! - Artifact paths declared by a deliverable contract are included, so a
//!   file owned only as a contract artifact is never mistaken for nobody's;
//!   they may be artifact-root-relative rather than repo-relative, and are
//!   listed as declared regardless. Erring toward "declared" is the
//!   fail-closed direction: it withholds the exemption rather than granting
//!   it wrongly.
//! - With no task universe nothing is stamped, and with no stamp the
//!   exemption is unavailable.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::v2::WorkflowV2FanoutItem;

/// Top-level input key of the stamp, alongside the base-commit stamp.
pub const PATH_OWNERSHIP_INPUT_KEY: &str = "path_ownership";

/// A path another task declares, and which task that is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredPath {
    pub path: String,
    pub owner_task: String,
}

/// The host's conclusion about who declares what, for one branch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathOwnership {
    /// Paths the task(s) under verification declare: this branch's scope.
    #[serde(default)]
    pub own_declared: Vec<String>,
    /// Every path any OTHER task declares, with its owner.
    #[serde(default)]
    pub declared_elsewhere: Vec<DeclaredPath>,
}

impl PathOwnership {
    /// Nothing to say, so nothing is stamped.
    pub fn is_empty(&self) -> bool {
        self.own_declared.is_empty() && self.declared_elsewhere.is_empty()
    }
}

/// Every repository or artifact path one task declares as its own: the files
/// it expects to change, the ones it appends to alongside other tasks, and
/// the artifacts its deliverable contracts name. The declared-files entries
/// are prose as often as paths, so they are read with the same parser the
/// author-wave planner compares them with.
pub fn declared_paths_of(task: &WorkflowV2TaskUniverseTask) -> BTreeSet<String> {
    task.files_expected_to_change
        .iter()
        .chain(task.shared_append_target_files.iter())
        .filter_map(|entry| crate::v2::script::declared_path(entry))
        .chain(
            task.deliverable_contracts
                .iter()
                .flat_map(|contract| {
                    [
                        Some(contract.artifact_path.clone()),
                        contract.registry_path.clone(),
                        contract.instance_source_path.clone(),
                    ]
                })
                .flatten(),
        )
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect()
}

/// The stamp for a branch claiming `claimed`. A path both this task and
/// another declare is this task's: the narrower answer, and the one that
/// refuses the exemption rather than granting it.
pub fn path_ownership_for(universe: &WorkflowV2TaskUniverse, claimed: &[String]) -> PathOwnership {
    let mine: BTreeSet<String> = universe
        .tasks
        .iter()
        .filter(|task| claimed.iter().any(|id| id == &task.canonical_task_id))
        .flat_map(declared_paths_of)
        .collect();
    let mut others: BTreeMap<String, String> = BTreeMap::new();
    for task in universe
        .tasks
        .iter()
        .filter(|task| !claimed.iter().any(|id| id == &task.canonical_task_id))
    {
        for path in declared_paths_of(task) {
            if mine.contains(&path) {
                continue;
            }
            others
                .entry(path)
                .or_insert_with(|| task.canonical_task_id.clone());
        }
    }
    PathOwnership {
        own_declared: mine.into_iter().collect(),
        declared_elsewhere: others
            .into_iter()
            .map(|(path, owner_task)| DeclaredPath { path, owner_task })
            .collect(),
    }
}

/// Stamp each branch with the conclusion for the tasks it claims. Universe
/// sourced, computed here so the universe itself never travels to a
/// read-only branch.
pub fn stamp_path_ownership_from_universe(
    mut items: Vec<WorkflowV2FanoutItem>,
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> Vec<WorkflowV2FanoutItem> {
    let Some(universe) = task_universe else {
        return items;
    };
    for item in &mut items {
        let claimed = crate::v2::branch_stamping::branch_canonical_task_ids(&item.input);
        if claimed.is_empty() {
            continue;
        }
        let ownership = path_ownership_for(universe, &claimed);
        if ownership.is_empty() {
            continue;
        }
        if let (Some(object), Ok(value)) =
            (item.input.as_object_mut(), serde_json::to_value(&ownership))
        {
            object.insert(PATH_OWNERSHIP_INPUT_KEY.to_string(), value);
        }
    }
    items
}

/// The stamp an input carries, if any.
pub fn stamped(input: &Value) -> Option<PathOwnership> {
    serde_json::from_value(input.get(PATH_OWNERSHIP_INPUT_KEY)?.clone()).ok()
}

#[cfg(test)]
#[path = "path_ownership_tests.rs"]
mod tests;
