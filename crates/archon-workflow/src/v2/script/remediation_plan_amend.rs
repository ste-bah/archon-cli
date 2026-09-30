//! Batch O: the remediation plan's scope amendments.
//!
//! A load-bearing file a finding names that no task declares is assigned to
//! a task through the run's recorded, chained scope-amendment transaction
//! (`task_scope_amendment`): to the task whose landing touched it, else the
//! task whose own text names it. The plan is then placed on the AMENDED
//! universe, so the finding goes to the grantee with the file in its scope.
//! Stored project data a finding names (under the project root) is granted
//! the same way, to the finding's own tasks when nothing else owns it: it
//! then lands through the audited project-input ledger with backups. A grant
//! already in force is never asked for again, so asking twice records
//! nothing new.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::super::WorkflowV2ResultStore;
use super::super::residual_paths::{deliverable_root, project_data, protected, provably_unowned};
use super::{explicitly_named, finding_text};
use crate::task_scope_amendment::{
    ScopeAmendment, ScopeAmendmentLedger, ScopeAmendmentRequest, ScopeGrantKind, ScopeGrantRoot,
    ScopePlanInputs, amend_task_scope, amended_universe_for_run, plan_scope_amendments,
};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::review_findings::task_ids_of;

/// What the amendment step leaves the plan: the universe to place findings
/// on, and each finding's project-data grants (project-relative paths, with
/// the tasks they were granted to).
pub(super) struct Amended {
    pub universe: Option<WorkflowV2TaskUniverse>,
    pub project_grants: Vec<BTreeMap<String, BTreeSet<String>>>,
}

/// The project root project data lives under: the nearest ancestor of the
/// task set that holds a `.archon` directory.
fn project_root(universe: &WorkflowV2TaskUniverse) -> Option<PathBuf> {
    let tasks = PathBuf::from(universe.source_roots.first()?);
    tasks
        .ancestors()
        .find(|dir| dir.join(".archon").is_dir())
        .map(Path::to_path_buf)
}

/// Record the grants these findings call for (see the module doc) and
/// return the amended universe, or no universe when the run has no
/// amendments (or no run root to record them in).
pub(super) fn amended_universe(
    findings: &[Value],
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
    store: Option<&WorkflowV2ResultStore>,
    cut: Option<&str>,
) -> Amended {
    let none = || Amended {
        universe: None,
        project_grants: vec![BTreeMap::new(); findings.len()],
    };
    let Some(run_root) = store.and_then(|store| store.root().parent().map(Path::to_path_buf))
    else {
        return none();
    };
    let project = project_root(universe);
    // Repository files, and project data under the project root: only the
    // project-data namespaces are listed (never the engine's run records).
    let data_files = match &project {
        Some(project) if project != root => project_data_files(project),
        _ => Vec::new(),
    };
    let data_named: Vec<BTreeSet<String>> = findings
        .iter()
        .map(|finding| named_project_data(&finding_text(finding), &data_files))
        .collect();
    let named: Vec<BTreeSet<String>> = findings
        .iter()
        .zip(&data_named)
        .map(|(finding, data)| {
            let mut files: BTreeSet<String> = explicitly_named(&finding_text(finding), root)
                .into_iter()
                .collect();
            files.extend(data.iter().cloned());
            files
        })
        .collect();
    let landed = store
        .map(|store| landed_files_by_task(store, cut))
        .unwrap_or_default();
    let empty = BTreeMap::new();
    let mut plan = plan_scope_amendments(&ScopePlanInputs {
        universe,
        repository_root: root,
        project_root: project.as_deref(),
        authored: &empty,
        landed_files_by_task: &landed,
        finding_named_files: &named,
        focused_test_files_by_task: &empty,
    });
    // A file a finding of a task names that no task declares is that task's
    // to fix: repository files through its patch, stored project data
    // through the audited project-input landing. Granted to the finding's
    // own tasks when no landing or text gave it to them.
    for ((finding, data), repo_named) in findings.iter().zip(&data_named).zip(&named) {
        let tasks: Vec<String> = task_ids_of(finding)
            .into_iter()
            .filter(|id| universe.tasks.iter().any(|t| &t.canonical_task_id == id))
            .collect();
        let unowned_repo = repo_named.iter().filter(|path| {
            !data.contains(*path) && !protected(path) && provably_unowned(universe, path, root)
        });
        for path in unowned_repo {
            for task in &tasks {
                if plan
                    .amendments
                    .iter()
                    .any(|g| &g.path == path && &g.task_id == task)
                {
                    continue;
                }
                plan.amendments.push(ScopeAmendment {
                    task_id: task.clone(),
                    path: path.clone(),
                    kind: if deliverable_root(path) {
                        ScopeGrantKind::DeliverableRoot
                    } else {
                        ScopeGrantKind::OwnerlessAssignment
                    },
                    root: ScopeGrantRoot::Repository,
                    shared_with: BTreeSet::new(),
                    evidence: "a review finding of the task names it".into(),
                });
            }
        }
        for path in data {
            for task in &tasks {
                if plan
                    .amendments
                    .iter()
                    .any(|g| &g.path == path && &g.task_id == task)
                {
                    continue;
                }
                plan.amendments.push(ScopeAmendment {
                    task_id: task.clone(),
                    path: path.clone(),
                    kind: ScopeGrantKind::OwnerlessAssignment,
                    root: ScopeGrantRoot::Project,
                    shared_with: BTreeSet::new(),
                    evidence: "a review finding of the task names this stored project data".into(),
                });
            }
        }
    }
    let ledger = ScopeAmendmentLedger::load(&run_root).ok();
    let in_force: BTreeSet<(String, String)> = ledger
        .iter()
        .flat_map(|ledger| ledger.set.grants.iter())
        .map(|grant| (grant.task_id.clone(), grant.path.clone()))
        .collect();
    let owed: Vec<ScopeAmendment> = plan
        .amendments
        .into_iter()
        .filter(|grant| !in_force.contains(&(grant.task_id.clone(), grant.path.clone())))
        .collect();
    if !owed.is_empty() {
        // Refused grants are recorded in the transaction's own log with
        // their reasons; the plan then places the finding without them.
        let _ = amend_task_scope(ScopeAmendmentRequest {
            run_root: &run_root,
            universe,
            repository_root: root,
            grants: owed,
            trigger: "remediation plan: files review findings name that no task declares",
        });
    }
    let granted: Vec<(String, String)> = ScopeAmendmentLedger::load(&run_root)
        .map(|ledger| {
            ledger
                .set
                .grants
                .into_iter()
                .filter(|grant| grant.root == ScopeGrantRoot::Project)
                .map(|grant| (grant.path, grant.task_id))
                .collect()
        })
        .unwrap_or_default();
    let project_grants = data_named
        .iter()
        .map(|data| {
            let mut by_path: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for (path, task) in &granted {
                if data.contains(path) {
                    by_path
                        .entry(path.clone())
                        .or_default()
                        .insert(task.clone());
                }
            }
            by_path
        })
        .collect();
    Amended {
        universe: amended_universe_for_run(&run_root, universe).ok().flatten(),
        project_grants,
    }
}

/// Every file under the project's data namespaces (`.archon/<ns>/...` that
/// [`project_data`] accepts), project-relative, sorted.
fn project_data_files(project: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(project.join(".archon")) else {
        return out;
    };
    let mut stack: Vec<(PathBuf, String)> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            (entry.path(), format!(".archon/{name}"))
        })
        .filter(|(_, rel)| project_data(&format!("{rel}/x")))
        .collect();
    while let Some((dir, rel)) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = format!("{rel}/{name}");
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push((entry.path(), path)),
                Ok(kind) if kind.is_file() => out.push(path),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

/// The project-data files `text` names: a token that is one of them, or
/// the path of one below its `.archon/` (with any leading directories), or
/// a trailing part of one of at least two segments naming exactly one.
fn named_project_data(text: &str, files: &[String]) -> BTreeSet<String> {
    let mut named = BTreeSet::new();
    if files.is_empty() {
        return named;
    }
    let tokens = text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
            )
    });
    for raw in tokens {
        let Some(token) = super::super::residual_paths::strip_location(raw) else {
            continue;
        };
        let token = match token.find(".archon/") {
            Some(at) => &token[at..],
            None => token,
        };
        if files.iter().any(|file| file == token) {
            named.insert(token.to_string());
            continue;
        }
        let suffix = format!("/{token}");
        let matching: Vec<&String> = files
            .iter()
            .filter(|file| file.ends_with(&suffix))
            .collect();
        if matching.len() == 1 {
            named.insert(matching[0].clone());
        }
    }
    named
}

/// The files each task's write landings changed, from the host's records:
/// each branch's outcome view paired with its item (the item carries the
/// evidence arrays), else the call's own list for the tasks it dispatched.
pub fn landed_files_by_task(
    store: &WorkflowV2ResultStore,
    cut: Option<&str>,
) -> BTreeMap<String, BTreeSet<String>> {
    let paths = |value: &Value| -> Vec<String> {
        value
            .get("files_changed")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                entry
                    .as_str()
                    .or_else(|| entry.get("path").and_then(Value::as_str))
                    .map(str::to_string)
            })
            .collect()
    };
    let mut landed: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for record in store.load_call_records().unwrap_or_default() {
        if record.call.write_mode.is_none() {
            continue;
        }
        // Only what had landed when the plan was first asked: a later
        // landing never moves a plan a resume replays.
        if cut.is_some_and(|cut| !cut.is_empty() && record.finished_at.as_str() > cut) {
            continue;
        }
        let data = &record.result.data;
        let outcomes = data
            .get("outcomes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let items = data
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if outcomes.is_empty() {
            let files = record.result.files_changed.iter().map(|f| f.path.clone());
            let files: Vec<String> = files.collect();
            for task in record
                .dispatched_items
                .iter()
                .flat_map(|i| i.canonical_task_ids.iter())
            {
                landed
                    .entry(task.clone())
                    .or_default()
                    .extend(files.iter().cloned());
            }
            continue;
        }
        for (at, view) in outcomes.iter().enumerate() {
            let mut files = paths(view);
            if let Some(item) = items.get(at) {
                files.extend(paths(item));
            }
            for task in task_ids_of(view) {
                landed
                    .entry(task)
                    .or_default()
                    .extend(files.iter().cloned());
            }
        }
    }
    landed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_data_is_named_by_full_path_or_a_unique_tail_and_engine_state_is_never_listed() {
        let dir = tempfile::tempdir().unwrap();
        for path in [
            ".archon/lab/data/snapshots/feed/X.json",
            ".archon/lab/data/datasets/x-1D/v1/metadata.json",
            ".archon/workflows/run-1/state.json",
        ] {
            let target = dir.path().join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, "{}").unwrap();
        }
        let files = project_data_files(dir.path());
        assert!(
            files.iter().all(|f| !f.starts_with(".archon/workflows/")),
            "{files:?}"
        );
        let text = "snapshots/feed/X.json:4 embeds foreign state; see `/abs/proj/.archon/lab/data/datasets/x-1D/v1/metadata.json` and data/metadata.json";
        let named = named_project_data(text, &files);
        assert_eq!(
            named.into_iter().collect::<Vec<_>>(),
            [
                ".archon/lab/data/datasets/x-1D/v1/metadata.json",
                ".archon/lab/data/snapshots/feed/X.json"
            ]
        );
    }
}
