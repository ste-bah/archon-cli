//! Batch O2 (ACC-H3): every file the acceptance stage grants a failing
//! check's unit goes through the run's chained scope-amendment ledger
//! (`task_scope_amendment::amend_task_scope`), never around it.
//!
//! The routing (`acceptance_routing::route_check`) decides who each file no
//! task declares goes to: its recorded owners (the ledger's ownership map),
//! the authors of the landing that broke the check, or the check's unit.
//! Here the host records those grants, one chained link per check with a
//! trigger naming it, so that:
//!
//! - the grant is host-validated (never engine or run state, the frozen
//!   task set, or a file the grantee forbids) and logged, and a refusal is
//!   recorded on the check as unwritable, with the ledger's reason -- the
//!   unit is never handed a file its write would be refused;
//! - the write fan-out plans with the amended universe
//!   (`write::scope_amendment_stamps`), so the declared-scope floor, the
//!   owner claims and the forbidden lists all see the grant;
//! - stored data -- a file the check's failure names under a data root the
//!   run's records declare (`DeclaredDataRoots`, Issue-223: inside or
//!   outside the repository, never by a path convention), or one of its
//!   implicated files the ledger lands through the project inputs -- in the
//!   project is a project grant (`routing.project_grants`): stamped on the
//!   grantees' branches and landed through the audited project-input
//!   landing, never a write target of the script's (the repository patch
//!   would skip it). Stored data in the repository is a granted file the
//!   patch lands.
//!
//! A grant already in force is never recorded again, so a round the host
//! re-runs on resume appends nothing. Every rule is generic: ids and paths
//! come from the round record, the universe, the ledger and the disk.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::super::acceptance_stage::{AcceptanceCheckRecordV1, AcceptanceRoundRecordV1};
use super::super::script::residual_paths::protected;
use super::AcceptanceRoutingV1;
use crate::task_scope_amendment::{
    DeclaredDataRoots, ScopeAmendment, ScopeAmendmentError, ScopeAmendmentLedger,
    ScopeAmendmentRequest, ScopeGrantKind, ScopeGrantRoot, amend_task_scope, ownership_map,
};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::write_coordinator::project_inputs::ProjectInputPolicy;

/// Each path the run's ledger names -> the tasks it names (ownership
/// records and write grants alike).
pub type OwnershipMap = BTreeMap<String, BTreeSet<String>>;

/// The run's recorded ownership; empty when the run amended nothing.
pub fn recorded_ownership(run_root: &Path) -> Result<OwnershipMap, ScopeAmendmentError> {
    ScopeAmendmentLedger::load(run_root).map(|ledger| ownership_map(&ledger.set))
}

/// The tasks of `universe` the ledger records as answering for `file`.
pub(super) fn recorded_owners(
    owned: &OwnershipMap,
    universe: &WorkflowV2TaskUniverse,
    file: &str,
) -> BTreeSet<String> {
    (owned.get(file).into_iter().flatten())
        .filter(|id| universe.tasks.iter().any(|t| &t.canonical_task_id == *id))
        .cloned()
        .collect()
}

/// The tasks a check's remediation unit is formed of, as the prelude forms
/// it: its owners, the landing that broke it, and its files' writers.
fn unit_of(check: &AcceptanceCheckRecordV1) -> BTreeSet<String> {
    let mut unit: BTreeSet<String> = check.owning_tasks.iter().cloned().collect();
    unit.extend(
        check
            .regressed_by
            .iter()
            .flat_map(|r| r.tasks.iter().cloned()),
    );
    unit.extend(
        check
            .routing
            .iter()
            .flat_map(|r| r.writer_tasks.iter().cloned()),
    );
    unit
}

/// Stored data `text` names: every path token (as written, or less a
/// trailing `:line:col`) that is a file under a data root the run's records
/// declare (`DeclaredDataRoots`), relative to the tree it lands in, with
/// that tree -- never engine or run state.
fn named_stored_data(text: &str, roots: &DeclaredDataRoots) -> BTreeMap<String, ScopeGrantRoot> {
    let separators = |c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '='
            )
    };
    let mut named = BTreeMap::new();
    for token in text.split(separators) {
        let token = token.trim_end_matches(['.', ',', ')', ']']);
        let bare = token.split(':').next().unwrap_or_default();
        for candidate in [token, bare] {
            let Some((relative, root)) = roots.locate(Path::new(candidate)) else {
                continue;
            };
            if !protected(&relative) {
                named.insert(relative, root);
                break;
            }
        }
    }
    named
}

/// Record every grant of `record`'s failing checks on the run's ledger (see
/// the module doc) and bring each check's routing in line with what the
/// ledger holds. An unreadable ledger is the round's operational error: the
/// write fan-out refuses to plan without it, so no grant may be implied.
pub fn record_routed_grants(
    run_root: &Path,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: &Path,
    record: &mut AcceptanceRoundRecordV1,
) {
    let Some(universe) = universe else {
        return;
    };
    let owned = match recorded_ownership(run_root) {
        Ok(owned) => owned,
        Err(error) => {
            record.operational_errors.push(format!(
                "the acceptance stage could not record its write grants: {error}"
            ));
            return;
        }
    };
    let policy = ProjectInputPolicy::for_landing(run_root);
    let project = policy.as_ref().map(|policy| policy.project.clone());
    let roots = (policy.as_ref()).map(|policy| DeclaredDataRoots::read(policy, universe, root));
    // Project data among the repository's files is the project's only when
    // the repository IS the project root.
    let same_root = project
        .as_ref()
        .is_some_and(|project| root.canonicalize().ok().as_deref() == Some(project.as_path()));
    let round = record.round;
    let mut errors = Vec::new();
    for check in &mut record.checks {
        if !check.ran_and_failed() {
            continue;
        }
        let unit = unit_of(check);
        let mut stored = BTreeMap::new();
        if let Some(roots) = &roots {
            let text = format!("{}\n{}", check.stderr_tail, check.stdout_tail);
            for (path, tree) in named_stored_data(&text, roots) {
                let owners = recorded_owners(&owned, universe, &path);
                let to = if owners.is_empty() {
                    unit.clone()
                } else {
                    owners
                };
                if !to.is_empty() {
                    stored.insert(path, (to.into_iter().collect::<Vec<_>>(), tree));
                }
            }
        }
        let routing = check
            .routing
            .get_or_insert_with(AcceptanceRoutingV1::default);
        for file in routing.granted_to.keys() {
            stored.remove(file);
        }
        let landing = check.regressed_by.as_ref();
        errors.extend(record_check(
            run_root,
            universe,
            root,
            Check {
                round,
                id: &check.check_id,
                unit: &unit,
                landing,
                same_root,
            },
            routing,
            stored,
        ));
        if routing == &AcceptanceRoutingV1::default() {
            check.routing = None;
        }
    }
    record.operational_errors.extend(errors);
}

/// What one check's grants are recorded against.
struct Check<'a> {
    round: u32,
    id: &'a str,
    unit: &'a BTreeSet<String>,
    /// The landing shown to break it, if one was.
    landing: Option<&'a crate::v2::acceptance_regression::AcceptanceRegressionV1>,
    /// Whether the repository IS the project root: only then can one of its
    /// files land as project data.
    same_root: bool,
}

/// Withdraw a grant of `file` from `routing`, recording why.
fn ungrant(routing: &mut AcceptanceRoutingV1, file: &str, why: String) {
    routing.granted_files.retain(|granted| granted != file);
    routing.granted_to.remove(file);
    routing.unwritable.push((file.to_string(), why));
}

/// One check's grants as one chained link, then its routing as the ledger
/// holds it. Returns the round errors it met: a ledger that cannot be read
/// withdraws every grant of the check (never implied).
fn record_check(
    run_root: &Path,
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
    check: Check<'_>,
    routing: &mut AcceptanceRoutingV1,
    stored: BTreeMap<String, (Vec<String>, ScopeGrantRoot)>,
) -> Vec<String> {
    let Check {
        round,
        id: check_id,
        unit,
        landing,
        same_root,
    } = check;
    let in_force = |ledger: &ScopeAmendmentLedger| -> BTreeMap<(String, String), ScopeGrantRoot> {
        (ledger.set.grants.iter())
            .filter(|grant| grant.kind.writable())
            .map(|grant| ((grant.task_id.clone(), grant.path.clone()), grant.root))
            .collect()
    };
    let stored_files: Vec<String> = stored.keys().cloned().collect();
    let stored_in_project: BTreeSet<String> = (stored.iter())
        .filter(|(_, (_, tree))| *tree == ScopeGrantRoot::Project)
        .map(|(file, _)| file.clone())
        .collect();
    let unreadable = |routing: &mut AcceptanceRoutingV1, error: String| {
        let files: Vec<String> = (routing.granted_to.keys())
            .chain(&stored_files)
            .cloned()
            .collect();
        for file in files {
            ungrant(
                routing,
                &file,
                format!("the host could not grant it: {error}"),
            );
        }
        vec![format!(
            "acceptance check {check_id}: its write grants could not be recorded: {error}"
        )]
    };
    let before = match ScopeAmendmentLedger::load(run_root) {
        Ok(ledger) => in_force(&ledger),
        Err(error) => return unreadable(routing, error.to_string()),
    };
    let evidence = format!("acceptance check {check_id} (round {round}) implicates it");
    let wanted: Vec<(String, Vec<String>, ScopeGrantRoot)> = (routing.granted_to.iter())
        .map(|(file, to)| (file.clone(), to.clone(), ScopeGrantRoot::Repository))
        .chain(
            stored
                .into_iter()
                .map(|(file, (to, root))| (file, to, root)),
        )
        .collect();
    let grants: Vec<ScopeAmendment> = (wanted.iter())
        .flat_map(|(file, to, root)| to.iter().map(move |task| (file, task, *root)))
        .filter(|(file, task, _)| !before.contains_key(&((*task).clone(), (*file).clone())))
        .map(|(file, task, root)| ScopeAmendment {
            task_id: task.clone(),
            path: file.clone(),
            // Stored data under a recorded data root is a deliverable root.
            kind: if stored_files.contains(file) {
                ScopeGrantKind::DeliverableRoot
            } else {
                ScopeGrantKind::OwnerlessAssignment
            },
            root,
            shared_with: BTreeSet::new(),
            evidence: evidence.clone(),
        })
        .collect();
    let mut refused: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if !grants.is_empty() {
        let trigger = format!(
            "acceptance check {check_id} (round {round}): write grants for its remediation unit {}",
            unit.iter().cloned().collect::<Vec<_>>().join("+")
        );
        match amend_task_scope(ScopeAmendmentRequest {
            run_root,
            universe,
            repository_root: root,
            grants,
            trigger: &trigger,
        }) {
            Ok(outcome) => {
                for (grant, why) in outcome.refused {
                    refused.entry(grant.path).or_default().push(why);
                }
            }
            Err(error) => {
                for (file, _, _) in &wanted {
                    refused
                        .entry(file.clone())
                        .or_default()
                        .push(error.to_string());
                }
            }
        }
    }
    let after = match ScopeAmendmentLedger::load(run_root) {
        Ok(ledger) => in_force(&ledger),
        Err(error) => return unreadable(routing, error.to_string()),
    };
    for (file, to, _) in wanted {
        let held: Vec<(String, ScopeGrantRoot)> = (to.iter())
            .filter_map(|task| {
                let root = after.get(&(task.clone(), file.clone()))?;
                Some((task.clone(), *root))
            })
            .collect();
        // M6: a file the blamed landing deleted, granted only to that
        // landing's own tasks: the ledger grants only a file that exists,
        // and restoring it is its authors' landing change, which the write
        // boundary already admitted to them. Anything else is withdrawn.
        let restore = landing.is_some_and(|landing| {
            landing.changed_files.contains(&file)
                && to.iter().all(|task| landing.tasks.contains(task))
        });
        if held.is_empty()
            && restore
            && !root.join(&file).exists()
            && routing.granted_to.contains_key(&file)
        {
            continue;
        }
        if held.is_empty() {
            let why = refused.get(&file).map_or(
                "the scope-amendment ledger holds no grant of it".into(),
                |why| why.join("; "),
            );
            ungrant(
                routing,
                &file,
                format!("the host could not grant it: {why}"),
            );
            routing.project_grants.remove(&file);
            continue;
        }
        let landed_as_data = held
            .iter()
            .any(|(_, root)| *root == ScopeGrantRoot::Project);
        if landed_as_data && !same_root && !stored_in_project.contains(&file) {
            // A repository file the ledger would land as project data, in a
            // repository that is not the project root: no landing can place
            // it where the check read it.
            ungrant(
                routing,
                &file,
                "a repository file the ledger lands as project data, but the repository is not the project root: no landing can place it".into(),
            );
            continue;
        }
        let tasks: Vec<String> = held.iter().map(|(task, _)| task.clone()).collect();
        if held
            .iter()
            .any(|(_, root)| *root == ScopeGrantRoot::Project)
        {
            // Landed through the project inputs: never a script target.
            routing.granted_files.retain(|granted| granted != &file);
            routing.granted_to.remove(&file);
            routing.project_grants.insert(file, tasks);
        } else {
            if stored_files.contains(&file) {
                // Stored data in the repository: a script target the patch
                // lands, whatever the plan-scope routing judged of it.
                routing
                    .unwritable
                    .retain(|(unwritable, _)| unwritable != &file);
                if !routing.granted_files.contains(&file) {
                    routing.granted_files.push(file.clone());
                }
            }
            routing.granted_to.insert(file, tasks);
        }
    }
    Vec::new()
}

#[cfg(test)]
#[path = "acceptance_grants_tests.rs"]
mod tests;
