//! A failing acceptance check is remediated by tasks that can WRITE the fix.
//!
//! The acceptance stage routed a failing check only to the tasks whose
//! `implements` list names it (`acceptance_stage::owning_tasks`), plus the
//! landing its regression search blamed (`acceptance_regression`). Neither
//! says who may write the file that broke: under the owner rule (Issue-121)
//! a routed implementer is refused another task's file, and a file no task
//! declares is outside every implementer's scope. Live on wf-0ddadd81 three
//! checks regressed in a file no task declares and were routed to tasks
//! whose scope roots did not reach it: two of the three units could not act.
//!
//! So each failing check also names the repository files its failure
//! implicates -- the failure locations in its own output
//! (`acceptance_signals`: panics, error-level diagnostics, stack frames;
//! never warnings, notes or lint listings), read from the end and bounded,
//! and every file the landing it regressed at changed -- and, for each, who
//! can write it. Nothing outside the plan's scope roots
//! (`acceptance_scope`: the product area the task set's declarations cover)
//! is ever routed or granted (Batch E2: the target repository may hold the
//! harness itself, and "no task declares it" is not "safe to hand over"):
//!
//! - a file outside the scope roots is recorded as unwritable, with why;
//! - a file a task declares (`residual_paths::owners`) routes the
//!   remediation to that task too, so the unit holds it;
//! - a file no task declares, PROVEN so (`provably_unowned`), not protected,
//!   and not forbidden to the unit's tasks, is granted to the unit (the
//!   Issue-117 expansion rule, without the residual round's exact lift: a
//!   granted file becomes a declared target, and a declared target overrides
//!   a forbidden entry, so a forbidden file is never granted);
//! - anything else is recorded as unwritable, with why, on the check --
//!   and so is the check's own source (what its frozen command runs: its
//!   program, the script or test it executes, a target it names by path or
//!   stem): a remediation fixes the implementation, never the check.
//!
//! A grant needs a unit: with no implementer, no blamed landing and no owner,
//! the tasks whose own text names the file (`TaskTexts::naming`) write it,
//! and are routed only when it is granted to them.
//!
//! Batch J: a file the blamed landing changed is its AUTHORS' -- the tasks of
//! the landing that broke the check, which wrote it in this run. Such a
//! file no task declares is granted when it is not forbidden to the
//! authors themselves: the owners' forbidden lists guard the owners' scope,
//! not a change another task made, so an owner forbidding it (live: the
//! owner forbade `src/command/**`, the author's change sat in it) never
//! withholds it. Nor do the plan's scope roots: the write boundary admitted
//! that change to its authors when it landed (under the write stage's own,
//! wider ceiling), and handing it back to them to restore reopens nothing
//! the run did not already open -- the Batch E2 roots still bound every file
//! read from a check's OUTPUT. A file another task DECLARES still goes only
//! through that task (the owner rule); protected paths, unreadable
//! declarations and the check's own sources apply as to every file. Every
//! changed file of the landing is implicated, unbounded, a file it deleted
//! included: the one that broke the check may be any of them. Error text with no `path:line` implicates nothing
//! (`acceptance_signals`); the regression search is that check's route.
//!
//! A failed check the rules above leave with no unit ([`blocking_rule`]) is
//! never raised and dropped: [`reroute`] reassigns it, and every other failed
//! check no task's `implements` names, to the tasks nearest the files its
//! failure implicates (`acceptance_reroute`), and fills its `owning_tasks`
//! so a round is sent for it. Only a universe with no task at all leaves a
//! check `blocked`. Every implicated file is routed: there is no cap.
//! Everything here is read from the host's records, the task universe and
//! the repository; agent text supplies path candidates only.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::acceptance_scope::PlanScopeRoots;
use super::acceptance_stage::AcceptanceRoundRecordV1;
use super::script::residual_paths::{
    TaskTexts, owners, protected, provably_unowned, residual_forbidden,
};
use crate::task_universe::WorkflowV2TaskUniverse;

#[path = "acceptance_reroute.rs"]
mod reroute_impl;
pub use reroute_impl::{nearest_owners, reroute};
#[path = "acceptance_routing_implicated.rs"]
mod implicated_impl;
use implicated_impl::implicated;
#[cfg(test)]
use implicated_impl::own_source;
#[path = "acceptance_grants.rs"]
mod grants_impl;
pub use grants_impl::{OwnershipMap, record_routed_grants, recorded_ownership};

/// Who can write what a failing check implicates.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceRoutingV1 {
    /// Repository files the failure implicates, in routing order.
    pub implicated_files: Vec<String>,
    /// Tasks the remediation is routed to so that it can write an
    /// implicated file: its owners, or for a granted file the tasks naming it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writer_tasks: Vec<String>,
    /// Implicated files no task declares, granted to the remediation unit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub granted_files: Vec<String>,
    /// Batch O2: who each granted file is granted to -- its recorded owners,
    /// the landing's authors, or the unit -- as the host records it on the
    /// run's scope-amendment chain (`acceptance_grants`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub granted_to: BTreeMap<String, Vec<String>>,
    /// Batch O2: stored project data the failure names, project-relative,
    /// with its grantees: stamped on their branches and landed through the
    /// audited project-input landing, never a write target of the script's.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub project_grants: BTreeMap<String, Vec<String>>,
    /// Issue-226: where each granted stored-data file lands, as the host
    /// states it to the unit: its tree (the project root, the repository or
    /// an external data root), its path there, and by which landing.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stored_data_landings: BTreeMap<String, String>,
    /// Implicated files no unit may be given, each with the reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unwritable: Vec<(String, String)>,
    /// Tasks the host reassigned the check to because no task's
    /// `implements` names it or no rule above gave it a unit ([`reroute`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reassigned_to: Vec<String>,
    /// Why the check was reassigned; empty when it was not.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reassign_reason: String,
}

impl AcceptanceRoutingV1 {
    /// Whether this routing gives the remediation somewhere to go.
    pub fn routes(&self) -> bool {
        !self.writer_tasks.is_empty()
    }

    /// The routing as a gap clause: `; it implicates ...`.
    pub fn describe(&self) -> String {
        let mut text = format!("; it implicates {}", self.implicated_files.join(", "));
        if !self.writer_tasks.is_empty() {
            text.push_str(&format!(
                "; routed also to {}, who can write them",
                self.writer_tasks.join(", ")
            ));
        }
        if !self.granted_files.is_empty() {
            text.push_str(&format!(
                "; granted {}, which no task declares",
                self.granted_files.join(", ")
            ));
        }
        for (file, why) in &self.unwritable {
            text.push_str(&format!("; {file} cannot be given to any unit: {why}"));
        }
        if !self.reassigned_to.is_empty() {
            text.push_str(&format!(
                "; reassigned to {} ({})",
                self.reassigned_to.join(", "),
                self.reassign_reason
            ));
        }
        text
    }
}

/// The routing of one failing check whose unit so far is `unit` (its
/// implementers and blamed landing's tasks), or `None` when it implicates no
/// repository file. Only a file inside the plan's scope roots (`scope`) ever
/// routes a writer or is granted -- or one the run's scope-amendment ledger
/// records an owner for (`owned`, Batch O2): it goes to that owner.
pub fn route_check(
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
    texts: &TaskTexts,
    scope: &PlanScopeRoots,
    owned: &OwnershipMap,
    check: &super::acceptance_stage::AcceptanceCheckRecordV1,
    command: &str,
) -> Option<AcceptanceRoutingV1> {
    let mut unwritable = Vec::new();
    let (files, landed) = implicated(check, root, command, &mut unwritable);
    if files.is_empty() && unwritable.is_empty() {
        return None;
    }
    let mut unit: BTreeSet<String> = check.owning_tasks.iter().cloned().collect();
    let authors: Vec<String> = (check.regressed_by.iter())
        .flat_map(|regression| regression.tasks.iter().cloned())
        .collect();
    unit.extend(authors.iter().cloned());
    let mut writers = BTreeSet::new();
    let mut unowned = Vec::new();
    let mut granted = Vec::new();
    let mut granted_to = BTreeMap::new();
    for file in &files {
        let declared = owners(universe, file, root);
        // Batch O2: who the run's ledger records as answering for it.
        let recorded = grants_impl::recorded_owners(owned, universe, file);
        // The landing's own change is its authors' whatever the plan's
        // scope roots say: the write boundary admitted it to them already.
        let authored = landed.contains(file) && !authors.is_empty();
        let why = if !authored && recorded.is_empty() && !scope.covers_on_disk(root, file) {
            "outside the plan's scope roots, so not the task set's to change".to_string()
        } else if !declared.is_empty() {
            writers.extend(declared);
            continue;
        } else if protected(file) {
            // Batch O: engine and run state or the frozen task set only; a
            // deliverable root (docs, task-set artifacts, project data) is
            // no longer protected and routes on like any unowned file.
            "engine or run state, which no unit may write".to_string()
        } else if !recorded.is_empty() {
            // Batch O2: a recorded owner answers for it, as the review
            // remediation's on-demand grants route it: the remediation goes
            // to them, and they are granted it (never one it forbids).
            let free: Vec<String> = (recorded.iter())
                .filter(|task| {
                    !residual_forbidden(universe, std::slice::from_ref(*task), &[]).matches(file)
                })
                .cloned()
                .collect();
            if free.is_empty() {
                let by: Vec<String> = recorded.into_iter().collect();
                format!("forbidden to {}, its recorded owner(s)", by.join(", "))
            } else {
                writers.extend(free.iter().cloned());
                granted_to.insert(file.clone(), free);
                granted.push(file.clone());
                continue;
            }
        } else if !provably_unowned(universe, file, root) {
            "no task declares it, but a task declaration cannot be read, so it is not provably unowned".to_string()
        } else if !authored {
            unowned.push(file.clone());
            continue;
        } else if residual_forbidden(universe, &authors, &[]).matches(file) {
            // The blamed landing's own change: its authors hold it.
            format!(
                "forbidden to {}, whose landing changed it",
                authors.join(", ")
            )
        } else {
            granted_to.insert(file.clone(), authors.clone());
            granted.push(file.clone());
            continue;
        };
        unwritable.push((file.clone(), why));
    }
    for file in unowned {
        // The unit that will hold it: everyone routed so far, else the tasks
        // whose own text names it.
        let mut holders: BTreeSet<String> = unit.union(&writers).cloned().collect();
        let named = holders.is_empty();
        if named {
            holders = texts.naming(&file, root);
        }
        if holders.is_empty() {
            unwritable.push((file, "no task declares it and none names it".into()));
            continue;
        }
        let ids: Vec<String> = holders.into_iter().collect();
        if residual_forbidden(universe, &ids, &[]).matches(&file) {
            unwritable.push((file, format!("forbidden to {}", ids.join(", "))));
            continue;
        }
        // Tasks found by naming hold the file only once it is theirs.
        if named {
            writers.extend(ids.iter().cloned());
        }
        granted_to.insert(file.clone(), ids);
        granted.push(file);
    }
    Some(AcceptanceRoutingV1 {
        implicated_files: files,
        writer_tasks: writers.into_iter().collect(),
        granted_files: granted,
        granted_to,
        unwritable,
        ..AcceptanceRoutingV1::default()
    })
}

/// The command a frozen check runs, as written; empty for a declarative
/// floor.
pub fn check_command(criterion: &crate::task_set_contract::AcceptanceCriterion) -> String {
    use crate::task_set_contract::AcceptanceCheck;
    match &criterion.check {
        AcceptanceCheck::Command { command, .. } => command.clone(),
        AcceptanceCheck::Floor { contract } => {
            contract.typed_verifier_command.clone().unwrap_or_default()
        }
    }
}

/// A check's routing as a gap clause; empty when it has none.
pub fn clause(check: &super::acceptance_stage::AcceptanceCheckRecordV1) -> String {
    check
        .routing
        .as_ref()
        .map_or(String::new(), AcceptanceRoutingV1::describe)
}

/// Route every failing, non-defect check of `record` (see the module docs)
/// against the frozen `criteria` it ran: what a check's command runs is the
/// check itself.
pub fn route_failures(
    universe: Option<&WorkflowV2TaskUniverse>,
    root: &Path,
    criteria: &[&crate::task_set_contract::AcceptanceCriterion],
    record: &mut AcceptanceRoundRecordV1,
) {
    route_failures_owned(universe, root, criteria, &OwnershipMap::new(), record);
}

/// [`route_failures`] with the run's recorded ownership
/// ([`recorded_ownership`]): a file no task declares but the ledger records
/// an owner for goes to that owner.
pub fn route_failures_owned(
    universe: Option<&WorkflowV2TaskUniverse>,
    root: &Path,
    criteria: &[&crate::task_set_contract::AcceptanceCriterion],
    owned: &OwnershipMap,
    record: &mut AcceptanceRoundRecordV1,
) {
    let Some(universe) = universe else {
        return;
    };
    let commands: BTreeMap<String, String> = criteria
        .iter()
        .map(|criterion| (criterion.id.clone(), check_command(criterion)))
        .collect();
    let texts = TaskTexts::read(universe, root);
    let scope = PlanScopeRoots::of(universe, root);
    for check in &mut record.checks {
        if !check.failing() || check.contract_defect {
            continue;
        }
        let text = commands
            .get(&check.check_id)
            .map(String::as_str)
            .unwrap_or_default();
        check.routing = route_check(universe, root, &texts, &scope, owned, check, text);
    }
}

/// Batch J: mark every failed check of `record` that no unit can fix with
/// the rule that blocks it (see [`blocking_rule`]). Run after routing;
/// [`reroute`] then reassigns every such check, so none stays blocked while
/// the universe has a task.
pub fn mark_blocked(record: &mut AcceptanceRoundRecordV1) {
    for check in &mut record.checks {
        check.blocked = check
            .ran_and_failed()
            .then(|| blocking_rule(check))
            .flatten();
    }
}

/// Why a check that RAN and FAILED can reach no unit able to fix it, if it
/// cannot:
///
/// - nothing routes it: no task's `implements` names it, no run landing was
///   shown to break it, and no file its failure implicates has a writer;
/// - it never held at any point of the run (the regression search observed
///   the base and every landing) and every file its failure implicates is
///   one no unit may be given, so its owners cannot make the fix either.
///
/// A check whose search was cut short is never blocked on the second rule:
/// it goes to its owners with the search's note.
pub fn blocking_rule(check: &super::acceptance_stage::AcceptanceCheckRecordV1) -> Option<String> {
    let search = (check.regression_search.as_ref())
        .map_or(String::new(), |search| format!(" ({})", search.note));
    let unwritable = |routing: &AcceptanceRoutingV1| {
        (routing.unwritable.iter())
            .map(|(file, why)| format!("{file}: {why}"))
            .collect::<Vec<_>>()
            .join("; ")
    };
    if !check.routed() {
        let files = match &check.routing {
            Some(routing) if !routing.unwritable.is_empty() => format!(
                "no file its failure implicates may be given to a unit ({})",
                unwritable(routing)
            ),
            _ => "its failure names no repository file a task could be granted".to_string(),
        };
        return Some(format!(
            "no remediation unit can be formed for it: no task's `implements` names it, no run landing was shown to break it{search}, and {files}"
        ));
    }
    let never_held = check.regressed_by.is_none()
        && (check.regression_search.as_ref()).is_some_and(|search| search.never_held);
    match &check.routing {
        Some(routing)
            if never_held
                && !routing.implicated_files.is_empty()
                && routing.writer_tasks.is_empty()
                && routing.granted_files.is_empty() =>
        {
            Some(format!(
                "it never held at any point of the run{search}, and every file its failure implicates is one no unit may be given: {}",
                unwritable(routing)
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "acceptance_routing_tests.rs"]
mod tests;
