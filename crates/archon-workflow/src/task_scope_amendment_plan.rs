//! The pure planner of scope amendments: which grants a run owes, computed
//! from the universe and the run's own facts, never from agent text alone.
//!
//! 1. **Declared-file restore.** Every single file a task declares (files
//!    expected to change, shared-append targets) that the authored script
//!    did not give it is restored to it.
//! 2. **Ownerless-file assignment.** Every load-bearing file -- one a task's
//!    landing touched, one a finding named, one a task's focused test runs --
//!    that is a repository file (or project data) no task declares is
//!    assigned: to the tasks whose landing touched it, else the tasks whose
//!    focused tests run it, else the tasks whose own text names it. A
//!    candidate grantee that forbids it by a wider pattern is passed over.
//!
//! What cannot be assigned is returned, with the reason, for the caller to
//! escalate: nothing is dropped.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::{ScopeAmendment, ScopeGrantKind, ScopeGrantRoot};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::script::residual_paths::{
    TaskTexts, deliverable_root, is_repo_file, owners, project_data, protected, provably_unowned,
    residual_forbidden,
};
use crate::v2::verification::path_ownership::{DeclaredPathForm, declared_path_form};

/// Everything the planner reads. Paths are repository-relative (project
/// data: project-relative), task ids canonical.
pub struct ScopePlanInputs<'a> {
    pub universe: &'a WorkflowV2TaskUniverse,
    pub repository_root: &'a Path,
    /// The project root project data lives under, when the run has one.
    pub project_root: Option<&'a Path>,
    /// Every path the authored script gave each task (targets and artifacts
    /// alike). A task absent here was not authored and is restored nothing.
    pub authored: &'a BTreeMap<String, BTreeSet<String>>,
    /// The files each task's landings changed.
    pub landed_files_by_task: &'a BTreeMap<String, BTreeSet<String>>,
    /// The files each finding names (`residual_paths::named_files`).
    pub finding_named_files: &'a [BTreeSet<String>],
    /// The files each task's focused tests run
    /// (`focused_test_targets::widenable`).
    pub focused_test_files_by_task: &'a BTreeMap<String, BTreeSet<String>>,
}

/// The grants owed, and every load-bearing file no task could be given.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeAmendmentPlan {
    pub amendments: Vec<ScopeAmendment>,
    pub unassigned: Vec<(String, String)>,
}

/// Plan every scope amendment the run owes (see the module doc). Pure: it
/// reads the filesystem only to tell files that exist, and writes nothing;
/// the grants still pass through `amend_task_scope`, which validates them.
pub fn plan_scope_amendments(inputs: &ScopePlanInputs<'_>) -> ScopeAmendmentPlan {
    let mut plan = ScopeAmendmentPlan::default();
    restore_declared(inputs, &mut plan);
    assign_ownerless(inputs, &mut plan);
    plan.amendments.sort();
    plan.amendments.dedup();
    plan
}

fn restore_declared(inputs: &ScopePlanInputs<'_>, plan: &mut ScopeAmendmentPlan) {
    let root = inputs.repository_root;
    for task in &inputs.universe.tasks {
        let Some(authored) = inputs.authored.get(&task.canonical_task_id) else {
            continue;
        };
        let authored: BTreeSet<String> = authored
            .iter()
            .filter_map(|path| repo_form(path, root))
            .collect();
        for entry in task
            .files_expected_to_change
            .iter()
            .chain(&task.shared_append_target_files)
        {
            let Some(path) = crate::v2::script::declared_path(entry)
                .and_then(|raw| repo_form(&raw, root))
                .and_then(|path| super::validate::clean_path(&path))
            else {
                continue;
            };
            if authored.contains(&path) || protected(&path) || root.join(&path).is_dir() {
                continue;
            }
            // The declaration is the evidence, deliverable root or not; the
            // path alone decides whether it lands as project data.
            plan.amendments.push(grant(
                &task.canonical_task_id,
                &path,
                ScopeGrantKind::DeclaredRestore,
                "declared under the task's files but absent from its authored scope",
            ));
        }
    }
}

fn assign_ownerless(inputs: &ScopePlanInputs<'_>, plan: &mut ScopeAmendmentPlan) {
    let root = inputs.repository_root;
    let universe = inputs.universe;
    let mut candidates: BTreeSet<String> = BTreeSet::new();
    candidates.extend(inputs.landed_files_by_task.values().flatten().cloned());
    candidates.extend(inputs.finding_named_files.iter().flatten().cloned());
    candidates.extend(
        inputs
            .focused_test_files_by_task
            .values()
            .flatten()
            .cloned(),
    );
    let texts = TaskTexts::read(universe, root);
    let holders = |map: &BTreeMap<String, BTreeSet<String>>, file: &str| -> BTreeSet<String> {
        map.iter()
            .filter(|(_, files)| files.contains(file))
            .map(|(task, _)| task.clone())
            .collect()
    };
    for raw in candidates {
        let Some(file) = super::validate::clean_path(&raw) else {
            plan.unassigned
                .push((raw, "not one clean relative file path".into()));
            continue;
        };
        if protected(&file) {
            plan.unassigned.push((
                file,
                "engine or run state or the frozen task set, which no grant opens".into(),
            ));
            continue;
        }
        let data = project_data(&file);
        let exists = if data {
            inputs
                .project_root
                .is_some_and(|project| project.join(&file).is_file())
        } else {
            is_repo_file(root, &file)
        };
        if !exists || !owners(universe, &file, root).is_empty() {
            continue;
        }
        if !provably_unowned(universe, &file, root) {
            plan.unassigned.push((
                file,
                "no task declares it, but a task declaration cannot be read".into(),
            ));
            continue;
        }
        let (tiers, evidence) = (
            [
                holders(inputs.landed_files_by_task, &file),
                holders(inputs.focused_test_files_by_task, &file),
                texts.naming(&file, root),
            ],
            [
                "a landing of the task changed it",
                "a focused test of the task runs it",
                "the task's own text names it",
            ],
        );
        let chosen = tiers.iter().zip(evidence).find_map(|(tasks, why)| {
            let allowed: BTreeSet<String> = tasks
                .iter()
                .filter(|task| {
                    !residual_forbidden(universe, std::slice::from_ref(*task), &[file.clone()])
                        .matches(&file)
                })
                .cloned()
                .collect();
            (!allowed.is_empty()).then_some((allowed, why))
        });
        let Some((tasks, why)) = chosen else {
            plan.unassigned.push((
                file,
                "no task declares it, and no task that landed, runs or names it may write it"
                    .into(),
            ));
            continue;
        };
        let kind = if deliverable_root(&file) {
            ScopeGrantKind::DeliverableRoot
        } else {
            ScopeGrantKind::OwnerlessAssignment
        };
        for task in tasks {
            plan.amendments.push(grant(&task, &file, kind, why));
        }
    }
}

fn grant(task: &str, path: &str, kind: ScopeGrantKind, evidence: &str) -> ScopeAmendment {
    ScopeAmendment {
        task_id: task.to_string(),
        path: path.to_string(),
        kind,
        root: if project_data(path) {
            ScopeGrantRoot::Project
        } else {
            ScopeGrantRoot::Repository
        },
        shared_with: BTreeSet::new(),
        evidence: evidence.to_string(),
    }
}

fn repo_form(raw: &str, root: &Path) -> Option<String> {
    match declared_path_form(raw, root) {
        DeclaredPathForm::Repo(path) => Some(path.trim_end_matches("/**").to_string()),
        _ => None,
    }
}
