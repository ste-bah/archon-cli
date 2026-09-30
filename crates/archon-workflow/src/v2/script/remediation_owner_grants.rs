//! Batch O: write grants on demand, from the ownership map.
//!
//! The set gate records who answers for each file no task declares
//! ([`ScopeGrantKind::Owner`]); none of that is write scope, so a task's
//! dispatch scope stays what it declares, what its script authored and its
//! declared-file restores. A unit may write an owned-but-undeclared file only
//! once something routed to that unit names the file: a review finding the
//! remediation plan places on it, or the unit's own verifier. The host then
//! grants the file -- to the unit's tasks that own it, else to the whole
//! unit -- through the chained scope-amendment transaction, one link per
//! unit, so every grant is logged against the unit that needed it. Nothing
//! is refused for want of a declaration; validation still refuses engine
//! state, the frozen task set and anything the grantee forbids, and logs
//! why.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use super::residual_paths::{deliverable_root, owners, project_data, protected, provably_unowned};
use super::{WorkflowV2CallRecord, WorkflowV2ResultStore, remediation_contract};
use crate::task_scope_amendment::{
    ScopeAmendment, ScopeAmendmentLedger, ScopeAmendmentRequest, ScopeAmendmentSet, ScopeGrantKind,
    ScopeGrantRoot, amend_task_scope, ownership_map,
};
use crate::task_universe::WorkflowV2TaskUniverse;

/// What one unit's routed work names.
pub struct UnitNamed<'a> {
    /// The unit's tasks.
    pub tasks: &'a BTreeSet<String>,
    /// The repository files the routed work names.
    pub files: &'a BTreeSet<String>,
    /// Who a named file no task declares or owns goes to (a finding's own
    /// tasks), when it is provably unowned; empty: nobody.
    pub unowned_to: &'a BTreeSet<String>,
    /// Recorded on each grant.
    pub evidence: &'a str,
    /// Keep grants the grantee already writes (a replayed plan reports
    /// them); `false`: only the new ones.
    pub keep_held: bool,
}

/// The write grants `named` is owed under `set` (see the module doc): only
/// for files no task declares, never engine state or project data (which
/// lands through its own grants), and never one the grantee already writes.
pub fn owed(
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
    set: &ScopeAmendmentSet,
    named: &UnitNamed<'_>,
) -> Vec<ScopeAmendment> {
    let owned = ownership_map(set);
    let writes: BTreeSet<(&str, &str)> = set
        .grants
        .iter()
        .filter(|grant| grant.kind.writable())
        .map(|grant| (grant.task_id.as_str(), grant.path.as_str()))
        .collect();
    let mut out = Vec::new();
    for file in named.files {
        if protected(file) || project_data(file) || !owners(universe, file, root).is_empty() {
            continue;
        }
        let grantees: BTreeSet<String> = match owned.get(file) {
            Some(by) => {
                let mine: BTreeSet<String> = named.tasks.intersection(by).cloned().collect();
                if mine.is_empty() {
                    named.tasks.clone()
                } else {
                    mine
                }
            }
            None if provably_unowned(universe, file, root) => named.unowned_to.clone(),
            None => continue,
        };
        for task in grantees {
            if !named.keep_held && writes.contains(&(task.as_str(), file.as_str())) {
                continue;
            }
            out.push(ScopeAmendment {
                task_id: task,
                path: file.clone(),
                kind: if deliverable_root(file) {
                    ScopeGrantKind::DeliverableRoot
                } else {
                    ScopeGrantKind::OwnerlessAssignment
                },
                root: ScopeGrantRoot::Repository,
                shared_with: BTreeSet::new(),
                evidence: named.evidence.to_string(),
            });
        }
    }
    out
}

/// Record one unit's owed grants as one chained amendment whose trigger
/// names the unit; the grants the host applied.
pub fn record_unit(
    run_root: &Path,
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
    unit: &str,
    grants: Vec<ScopeAmendment>,
) -> Vec<ScopeAmendment> {
    if grants.is_empty() {
        return Vec::new();
    }
    let trigger = format!("on-demand write grant for unit {unit}: routed work names the file");
    amend_task_scope(ScopeAmendmentRequest {
        run_root,
        universe,
        repository_root: root,
        grants,
        trigger: &trigger,
    })
    .map(|outcome| outcome.applied)
    .unwrap_or_default()
}

/// A verifier that left an id open and names an owned-but-undeclared file:
/// the file is granted to its unit, so the unit's next round may write it.
/// Read from the verifier's own record; a verifier that closed everything
/// grants nothing.
pub fn grant_verifier_named(
    record: &WorkflowV2CallRecord,
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> Vec<ScopeAmendment> {
    let (Some(universe), Some(root), Some(run_root)) = (universe, root, store.root().parent())
    else {
        return Vec::new();
    };
    let Some(read) = super::remediation_dispositions::reading(record) else {
        return Vec::new();
    };
    if read.values().all(Result::is_ok) {
        return Vec::new();
    }
    let Some(contract) = remediation_contract(&record.call) else {
        return Vec::new();
    };
    let tasks = unit_tasks(contract);
    if tasks.is_empty() {
        return Vec::new();
    }
    let mut text = record.result.summary.clone();
    text.push('\n');
    text.push_str(&super::remediation_plan::finding_text(&record.result.data));
    let files: BTreeSet<String> = super::remediation_plan::explicitly_named(&text, root)
        .into_iter()
        .collect();
    let Ok(ledger) = ScopeAmendmentLedger::load(run_root) else {
        return Vec::new();
    };
    let grants = owed(
        universe,
        root,
        &ledger.set,
        &UnitNamed {
            tasks: &tasks,
            files: &files,
            unowned_to: &BTreeSet::new(),
            evidence: "the unit's verifier names it",
            keep_held: false,
        },
    );
    let unit = contract
        .get("unit")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| tasks.iter().cloned().collect::<Vec<_>>().join("+"));
    record_unit(
        run_root,
        universe,
        root,
        &format!("{unit} (verifier)"),
        grants,
    )
}

/// A remediation contract's tasks: `taskIds`, else `taskId`.
fn unit_tasks(contract: &Value) -> BTreeSet<String> {
    let listed: BTreeSet<String> = contract
        .get("taskIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if !listed.is_empty() {
        return listed;
    }
    contract
        .get("taskId")
        .and_then(Value::as_str)
        .map(|id| BTreeSet::from([id.trim().to_string()]))
        .unwrap_or_default()
}

/// Each finding's applied on-demand grants, (task, path), for the plan view.
pub type GrantsByFinding = Vec<Vec<(String, String)>>;

/// Group per-finding owed grants by unit and record one link per unit; the
/// applied grants of each finding.
pub fn record_by_unit(
    run_root: &Path,
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
    per_finding: Vec<(BTreeSet<String>, Vec<ScopeAmendment>)>,
) -> GrantsByFinding {
    // Grants already in force (an earlier ask of the same plan) are
    // reported, never recorded again.
    let in_force: BTreeSet<(String, String)> = ScopeAmendmentLedger::load(run_root)
        .map(|ledger| {
            ledger
                .set
                .grants
                .into_iter()
                .filter(|grant| grant.kind.writable())
                .map(|grant| (grant.task_id, grant.path))
                .collect()
        })
        .unwrap_or_default();
    let mut units: BTreeMap<BTreeSet<String>, Vec<ScopeAmendment>> = BTreeMap::new();
    let mut seen: BTreeSet<(String, String)> = in_force.clone();
    for (tasks, grants) in &per_finding {
        for grant in grants {
            if seen.insert((grant.task_id.clone(), grant.path.clone())) {
                units.entry(tasks.clone()).or_default().push(grant.clone());
            }
        }
    }
    let mut applied: BTreeSet<(String, String)> = BTreeSet::new();
    for (tasks, grants) in units {
        let unit = tasks.iter().cloned().collect::<Vec<_>>().join("+");
        for grant in record_unit(run_root, universe, root, &unit, grants) {
            applied.insert((grant.task_id, grant.path));
        }
    }
    per_finding
        .into_iter()
        .map(|(_, grants)| {
            grants
                .into_iter()
                .map(|grant| (grant.task_id, grant.path))
                .filter(|key| applied.contains(key) || in_force.contains(key))
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_universe::WorkflowV2TaskUniverseTask;

    fn grant(task: &str, path: &str, kind: ScopeGrantKind) -> ScopeAmendment {
        ScopeAmendment {
            task_id: task.into(),
            path: path.into(),
            kind,
            root: ScopeGrantRoot::Repository,
            shared_with: BTreeSet::new(),
            evidence: String::new(),
        }
    }

    #[test]
    fn a_named_owned_file_goes_to_the_units_owners_and_nothing_unnamed_or_held_is_owed() {
        let dir = tempfile::tempdir().unwrap();
        for path in [
            "src/a.rs",
            "src/owned.rs",
            "src/shared.rs",
            "src/unnamed.rs",
        ] {
            let target = dir.path().join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, "").unwrap();
        }
        let task = |id: &str, owns: &str| WorkflowV2TaskUniverseTask {
            canonical_task_id: id.into(),
            files_expected_to_change: vec![owns.into()],
            ..Default::default()
        };
        let universe = WorkflowV2TaskUniverse {
            schema_version: "test".into(),
            source_roots: Vec::new(),
            tasks: vec![task("T-A", "src/a.rs"), task("T-B", "src/b.rs")],
        };
        let set = ScopeAmendmentSet {
            grants: vec![
                grant("T-A", "src/owned.rs", ScopeGrantKind::Owner),
                grant("T-B", "src/shared.rs", ScopeGrantKind::Owner),
                grant("T-A", "src/unnamed.rs", ScopeGrantKind::Owner),
            ],
            ..ScopeAmendmentSet::default()
        };
        let unit = BTreeSet::from(["T-A".to_string(), "T-B".to_string()]);
        let files = BTreeSet::from([
            "src/a.rs".to_string(),
            "src/owned.rs".to_string(),
            "src/shared.rs".to_string(),
        ]);
        let nobody = BTreeSet::new();
        let named = |keep_held| UnitNamed {
            tasks: &unit,
            files: &files,
            unowned_to: &nobody,
            evidence: "named",
            keep_held,
        };
        let got: Vec<(String, String)> = owed(&universe, dir.path(), &set, &named(false))
            .into_iter()
            .map(|g| (g.task_id, g.path))
            .collect();
        // Declared files need nothing; each owned file goes to its owner in
        // the unit; the unnamed one is owed to nobody.
        assert_eq!(
            got,
            [
                ("T-A".to_string(), "src/owned.rs".to_string()),
                ("T-B".to_string(), "src/shared.rs".to_string()),
            ]
        );
        // Once written, a grant is not owed again (unless a replay asks to
        // see it).
        let mut held = set.clone();
        held.grants.push(grant(
            "T-A",
            "src/owned.rs",
            ScopeGrantKind::OwnerlessAssignment,
        ));
        assert_eq!(owed(&universe, dir.path(), &held, &named(false)).len(), 1);
        assert_eq!(owed(&universe, dir.path(), &held, &named(true)).len(), 2);
    }
}
