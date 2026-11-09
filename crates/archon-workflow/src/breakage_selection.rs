//! Pure selection, quotas and evidence attribution.
use super::*;
use crate::base_snapshot::{BaseSnapshot, SnapshotEntry, SnapshotKind, snapshot_path_key};
use crate::baseline_results::{BaselineOutcome, check_digest};
use crate::task_set_contract::{AcceptanceContract, AcceptanceLock};
use crate::v2::requirement_classes::{
    RequirementClass, RequirementClassification, RequirementEntry,
};
use std::collections::BTreeSet;
use std::path::Path;

pub fn validate(
    set: &MutantSet,
    commit: &str,
    pairs: &[(String, String)],
) -> Result<(Vec<SuppliedMutant>, Vec<String>), &'static str> {
    if set.schema != BREAKAGE_SCHEMA {
        return Err("mutant_set_schema");
    }
    if set.base_commit != commit {
        return Err("mutant_commit_mismatch");
    }
    let mut accepted = Vec::new();
    let mut refusals = Vec::new();
    let mut seen = BTreeSet::new();
    let mut quotas = BTreeMap::new();
    for m in &set.mutants {
        let pair = (&m.task_id, &m.requirement_id);
        let quota = quotas.entry((pair, m.kind)).or_insert(0);
        let code = if !pairs.iter().any(|(task, id)| (task, id) == pair) {
            Some("mutant_unknown_pair")
        } else if m.patch.len() > 256 * 1024 {
            Some("patch_too_large")
        } else if m.patch.is_empty() {
            Some("patch_empty")
        } else if seen.contains(&(pair, &m.patch)) {
            Some("patch_duplicate")
        } else if *quota
            >= if m.kind == MutantKind::Behaviour {
                5
            } else {
                1
            }
        {
            Some("mutant_quota_exceeded")
        } else {
            None
        };
        if let Some(code) = code {
            refusals.push(format!("{}: {code}", m.mutant_id()));
        } else {
            *quota += 1;
            seen.insert((pair, &m.patch));
            accepted.push(m.clone());
        }
    }
    Ok((accepted, refusals))
}
pub fn passing_checks(
    entry: &RequirementEntry,
    contract: &AcceptanceContract,
    lock: &AcceptanceLock,
) -> Vec<String> {
    if entry.class != RequirementClass::CandidateDone {
        return Vec::new();
    }
    entry
        .checks
        .iter()
        .filter(|(id, outcome)| {
            **outcome == Some(BaselineOutcome::Passed)
                && contract
                    .acceptance
                    .iter()
                    .chain(&contract.supplementary)
                    .any(|c| {
                        &c.id == *id
                            && lock
                                .baseline_results
                                .get(*id)
                                .is_some_and(|r| r.check_digest == check_digest(&c.check))
                    })
        })
        .map(|(id, _)| id.clone())
        .collect()
}
pub fn keys(
    entry: &RequirementEntry,
    snapshot: &BaseSnapshot,
    root: &Path,
    shared: bool,
) -> Vec<(String, SnapshotKind)> {
    entry
        .deliverables
        .iter()
        .filter(|d| d.shared == shared)
        .filter_map(|d| {
            let key = snapshot_path_key(root, &d.artifact_path);
            match snapshot.entries.get(&key) {
                Some(SnapshotEntry::Tracked { kind, .. }) => Some((key, *kind)),
                _ => None,
            }
        })
        .collect()
}
pub fn contains(keys: &[(String, SnapshotKind)], path: &str) -> bool {
    keys.iter().any(|(key, kind)| {
        key == path || *kind == SnapshotKind::Tree && path.starts_with(&format!("{key}/"))
    })
}
pub fn attribution(
    touched: &[String],
    countable: &[(String, SnapshotKind)],
    shared: &[(String, SnapshotKind)],
) -> Option<NotCounted> {
    if touched.iter().any(|p| contains(shared, p)) {
        Some(NotCounted::TouchesShared)
    } else if touched.is_empty() || touched.iter().any(|p| !contains(countable, p)) {
        Some(NotCounted::TouchesOutsideDeliverables)
    } else {
        None
    }
}
pub fn estimate(
    classes: &RequirementClassification,
    contract: &AcceptanceContract,
    lock: &AcceptanceLock,
    root: &Path,
) -> u64 {
    let Some(snapshot) = &classes.base_snapshot else {
        return 0;
    };
    let checks: Vec<_> = classes
        .entries
        .iter()
        .filter(|e| !keys(e, snapshot, root, false).is_empty())
        .flat_map(|e| passing_checks(e, contract, lock))
        .collect();
    2 * checks.iter().collect::<BTreeSet<_>>().len() as u64 + 6 * checks.len() as u64
}
