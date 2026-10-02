//! Batch O: the remediation plan's scope amendments.
//!
//! Every load-bearing file no task declares -- one a landing touched, a
//! finding named, or a task's declared code references -- gets an OWNER
//! through the run's recorded, chained scope-amendment transaction
//! (`task_scope_amendment`): the set gate's ownership map, never write
//! scope. The plan routes each finding on the ownership map (so a finding
//! naming an owned file goes to its owner), and only then grants the unit
//! it lands in the files that finding names (`remediation_owner_grants`),
//! one logged link per unit. The unit's write targets are read from the
//! write grants alone.
//! Stored data a finding names (under the project root: its `.archon/`
//! data namespaces or any data root the run's records declare) is granted
//! the same way, to the finding's own tasks when nothing else owns it: it
//! then lands through the audited project-input ledger with backups (one
//! under a declared external data root the run's policy allowlists,
//! Issue-226, as an `External` grant, in that root). A grant
//! already in force is never asked for again, so asking twice records
//! nothing new.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::super::WorkflowV2ResultStore;
use super::super::remediation_owner_grants::{UnitNamed, owed as owed_grants, record_by_unit};
use super::super::residual_paths::project_data;
use super::{explicitly_named, finding_text};
use crate::task_scope_amendment::{
    DeclaredDataRoots, ScopeAmendment, ScopeAmendmentLedger, ScopeAmendmentRequest, ScopeGrantKind,
    ScopeGrantRoot, ScopePlanInputs, amend_task_scope, amended_universe_for_run,
    plan_scope_amendments, routing_universe,
};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::review_finding_ids::finding_id_of;
use crate::v2::review_findings::task_ids_of;
use crate::write_coordinator::project_inputs::ProjectInputPolicy;

/// What the amendment step leaves the plan: the universe to place findings
/// on, and each finding's project-data grants (project-relative paths, with
/// the tasks they were granted to).
pub(super) struct Amended {
    /// The write universe: declared files and write grants only.
    pub universe: Option<WorkflowV2TaskUniverse>,
    /// The routing universe: ownership records too.
    pub routing: Option<WorkflowV2TaskUniverse>,
    pub project_grants: Vec<BTreeMap<String, BTreeSet<String>>>,
    /// Each finding's on-demand write grants, (task, path).
    pub owner_grants: super::super::remediation_owner_grants::GrantsByFinding,
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
        routing: None,
        project_grants: vec![BTreeMap::new(); findings.len()],
        owner_grants: vec![Vec::new(); findings.len()],
    };
    let Some(run_root) = store.and_then(|store| store.root().parent().map(Path::to_path_buf))
    else {
        return none();
    };
    let project = project_root(universe);
    // Repository files, and stored data under the project root: the
    // project-data namespaces and every data root the run's records declare
    // (the ledger's own list, Issue-223) -- never the engine's run records.
    let policy = ProjectInputPolicy::for_landing(&run_root);
    let data_project =
        (policy.as_ref().map(|policy| policy.project.clone())).or_else(|| project.clone());
    let separate = |project: &PathBuf| {
        root.canonicalize()
            .map(archon_shell::paths::plain)
            .ok()
            .as_ref()
            != Some(project)
    };
    let roots = (policy.as_ref()).map(|policy| DeclaredDataRoots::read(policy, universe, root));
    let data_files = match &data_project {
        Some(data_project) if data_project != root && separate(data_project) => {
            let mut files = project_data_files(data_project);
            files.extend(roots.iter().flat_map(DeclaredDataRoots::project_files));
            files.sort();
            files.dedup();
            files
        }
        _ => Vec::new(),
    };
    let data_named: Vec<BTreeSet<String>> = findings
        .iter()
        .map(|finding| {
            let text = finding_text(finding);
            named_project_data(&text, data_project.as_deref(), &data_files)
        })
        .collect();
    let external_named: Vec<BTreeSet<String>> = (findings.iter())
        .map(|finding| named_external_data(&finding_text(finding), roots.as_ref()))
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
    // Stored project data a finding of a task names is that task's to fix,
    // through the audited project-input landing: written only when a finding
    // names it. (Repository files are granted per unit, below.)
    for ((finding, data), external) in findings.iter().zip(&data_named).zip(&external_named) {
        let tasks: Vec<String> = task_ids_of(finding)
            .into_iter()
            .filter(|id| universe.tasks.iter().any(|t| &t.canonical_task_id == id))
            .collect();
        let stored = (data.iter().map(|path| (path, ScopeGrantRoot::Project)))
            .chain(external.iter().map(|path| (path, ScopeGrantRoot::External)));
        for (path, tree) in stored {
            for task in &tasks {
                // An ownership record of the pair is superseded: the write
                // grant implies it.
                if plan
                    .amendments
                    .iter()
                    .any(|g| &g.path == path && &g.task_id == task && g.kind.writable())
                {
                    continue;
                }
                plan.amendments.push(ScopeAmendment {
                    task_id: task.clone(),
                    path: path.clone(),
                    kind: ScopeGrantKind::OwnerlessAssignment,
                    root: tree,
                    shared_with: BTreeSet::new(),
                    evidence: "a review finding of the task names this stored project data".into(),
                });
            }
        }
    }
    let ledger = ScopeAmendmentLedger::load(&run_root).ok();
    let held = |writes: bool| -> BTreeSet<(String, String)> {
        ledger
            .iter()
            .flat_map(|ledger| ledger.set.grants.iter())
            .filter(|grant| !writes || grant.kind.writable())
            .map(|grant| (grant.task_id.clone(), grant.path.clone()))
            .collect()
    };
    let (any, writes) = (held(false), held(true));
    let owed: Vec<ScopeAmendment> = plan
        .amendments
        .into_iter()
        .filter(|grant| {
            let key = (grant.task_id.clone(), grant.path.clone());
            if grant.kind.writable() {
                !writes.contains(&key)
            } else {
                !any.contains(&key)
            }
        })
        .collect();
    if !owed.is_empty() {
        // Refused grants are recorded in the transaction's own log with
        // their reasons; the plan then places the finding without them.
        let _ = amend_task_scope(ScopeAmendmentRequest {
            run_root: &run_root,
            universe,
            repository_root: root,
            grants: owed,
            trigger: "remediation plan: the ownership map of files no task declares, and stored project data findings name",
        });
    }
    // Route each finding on the ownership map, then grant its unit the
    // repository files it names.
    let set = ScopeAmendmentLedger::load(&run_root)
        .map(|ledger| ledger.set)
        .unwrap_or_default();
    let routing = routing_universe(universe, &set);
    let texts = super::super::residual_paths::TaskTexts::read(&routing, root);
    let known: BTreeSet<&str> = universe
        .tasks
        .iter()
        .map(|task| task.canonical_task_id.as_str())
        .collect();
    let per_finding: Vec<(BTreeSet<String>, Vec<ScopeAmendment>)> = findings
        .iter()
        .zip(&named)
        .zip(&data_named)
        .map(|((finding, named), data)| {
            let placed = super::place(finding, &routing, root, &texts, &known);
            let tasks: BTreeSet<String> = placed.tasks.into_iter().collect();
            let own: BTreeSet<String> = task_ids_of(finding)
                .into_iter()
                .filter(|id| known.contains(id.as_str()))
                .collect();
            let files: BTreeSet<String> = named.difference(data).cloned().collect();
            let evidence = format!(
                "finding {} names it, routed to the task's unit",
                finding_id_of(finding)
            );
            let grants = owed_grants(
                universe,
                root,
                &set,
                &UnitNamed {
                    tasks: &tasks,
                    files: &files,
                    unowned_to: &own,
                    evidence: &evidence,
                    keep_held: true,
                },
            );
            (tasks, grants)
        })
        .collect();
    let owner_grants = record_by_unit(&run_root, universe, root, per_finding);
    let granted: Vec<(String, String)> = ScopeAmendmentLedger::load(&run_root)
        .map(|ledger| {
            ledger
                .set
                .grants
                .into_iter()
                .filter(|grant| grant.root != ScopeGrantRoot::Repository && grant.kind.writable())
                .map(|grant| (grant.path, grant.task_id))
                .collect()
        })
        .unwrap_or_default();
    let project_grants = (data_named.iter().zip(&external_named))
        .map(|(data, external)| {
            let mut by_path: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for (path, task) in &granted {
                if data.contains(path) || external.contains(path) {
                    by_path
                        .entry(path.clone())
                        .or_default()
                        .insert(task.clone());
                }
            }
            by_path
        })
        .collect();
    let routing = ScopeAmendmentLedger::load(&run_root)
        .ok()
        .map(|ledger| routing_universe(universe, &ledger.set));
    Amended {
        universe: amended_universe_for_run(&run_root, universe).ok().flatten(),
        routing,
        project_grants,
        owner_grants,
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

/// The stored-data files `text` names: a token that is one of them, by its
/// absolute path under `project` or relative to it, or the path of one
/// below its `.archon/` (with any leading directories), or a trailing part
/// of one of at least two segments naming exactly one.
fn named_project_data(text: &str, project: Option<&Path>, files: &[String]) -> BTreeSet<String> {
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
        // An absolute path under the project names its project-relative
        // file, every link resolved.
        let under_project =
            project
                .filter(|_| Path::new(token).is_absolute())
                .and_then(|project| {
                    let path = Path::new(token);
                    let path = path
                        .canonicalize()
                        .map(archon_shell::paths::plain)
                        .unwrap_or_else(|_| path.to_path_buf());
                    // Repository-relative paths are always `/`-separated; strip
                    // the Windows `\` so the match below finds them (Issue-234).
                    Some(
                        path.strip_prefix(project)
                            .ok()?
                            .to_str()?
                            .replace('\\', "/"),
                    )
                });
        let token = match (&under_project, token.find(".archon/")) {
            (Some(rel), _) => rel.as_str(),
            (None, Some(at)) => &token[at..],
            (None, None) => token,
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

/// Issue-226: the files under a declared external data root `text` names
/// by absolute path, every link resolved (`DeclaredDataRoots::locate`).
fn named_external_data(text: &str, roots: Option<&DeclaredDataRoots>) -> BTreeSet<String> {
    let Some(roots) = roots else {
        return BTreeSet::new();
    };
    let separator = |c: char| c.is_whitespace() || "`\"'()[]{},;".contains(c);
    (text.split(separator))
        .filter_map(super::super::residual_paths::strip_location)
        .filter(|token| Path::new(token).is_absolute())
        .filter_map(|token| match roots.locate(Path::new(token))? {
            (path, ScopeGrantRoot::External) => Some(path),
            _ => None,
        })
        .collect()
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
        let named = named_project_data(text, None, &files);
        assert_eq!(
            named.into_iter().collect::<Vec<_>>(),
            [
                ".archon/lab/data/datasets/x-1D/v1/metadata.json",
                ".archon/lab/data/snapshots/feed/X.json"
            ]
        );
    }
}
