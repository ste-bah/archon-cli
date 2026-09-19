//! Issue-51: an attempt lands only the paths that need a fresh verdict.
//!
//! Every other declared path carries its last record forward verbatim, so an
//! attempt at an unchanged tree costs one agent call per new path, not one per
//! declared path (live: 705–707 re-landings, 2–3 hours, for 0–2 new paths).
//! The ledger still accepts one full report per attempt; only its assembly
//! changes. Nothing here reads the live checkout: `changed` comes from the
//! sealed views (`super::changes`).
use super::{AuditContract, AuditRecord, AuditReport, ledger::Reassessment, runtime::AuditState};
use std::collections::{BTreeMap, BTreeSet};

/// What this attempt asks the assessor for, and what it reuses.
#[derive(Clone, Debug, Default)]
pub struct CarryPlan {
    /// Declared paths that need a fresh verdict at this snapshot, sorted.
    pub delta: Vec<String>,
    /// Records reused verbatim from the last report, by declared path.
    pub carried: BTreeMap<String, AuditRecord>,
}
impl CarryPlan {
    /// The delta is: `added` (not declared before this call), paths named by a
    /// pending reassessment, declared paths with no record in the last report,
    /// and — at a new snapshot identity only — paths whose content changed,
    /// paths one of whose equivalents changed, and paths whose obligation is
    /// open (`resolved_snapshot` is `None`: a negative verdict, or an apply
    /// credited but not yet re-judged, which another file's edit may satisfy).
    /// Every other declared path carries its last record forward. At the same
    /// identity there is no new evidence, so an open obligation with a record
    /// at this snapshot is carried too; only a reassessment re-opens it.
    /// `changed` must be empty at the same identity (as `changes::between`).
    pub fn build(state: &AuditState, snapshot: &str, added: &BTreeSet<String>, pending: &[Reassessment], changed: &BTreeSet<String>) -> Self {
        let disputed = pending.iter().map(|r| r.declared_path.as_str()).collect::<BTreeSet<_>>();
        let assessed = state.snapshot.as_ref().map(|s| s.identity.as_str());
        let same = assessed == Some(snapshot);
        let last = state.ledger.history.last().filter(|report| Some(report.snapshot.as_str()) == assessed)
            .map(|report| report.records.iter().map(|r| (r.declared_path.as_str(), r)).collect::<BTreeMap<_, _>>()).unwrap_or_default();
        let open = |path: &str| state.ledger.obligations.get(path).is_some_and(|o| o.resolved_snapshot.is_none());
        let mut plan = Self::default();
        for path in &state.declared_paths {
            let record = last.get(path.as_str()).filter(|record| !added.contains(path) && !disputed.contains(path.as_str())
                && (same || (!changed.contains(path) && !record.equivalents.iter().any(|e| changed.contains(e)) && !open(path))));
            match record {
                Some(record) => { plan.carried.insert(path.clone(), (*record).clone()); }
                None => plan.delta.push(path.clone()),
            }
        }
        plan
    }
    /// The contract the assessor and the landing see: the delta only.
    pub fn contract(&self, snapshot: &str) -> AuditContract {
        AuditContract { schema_version: 1, snapshot: snapshot.into(), declared_paths: self.delta.clone() }
    }
    /// The full report the ledger accepts: carried records plus this attempt's
    /// delta records, one per declared path, sorted by path. The caller still
    /// validates it under the full contract and against the sealed files.
    pub fn merge(&self, delta: &AuditReport) -> AuditReport {
        let mut records = self.carried.clone();
        for record in &delta.records { records.insert(record.declared_path.clone(), record.clone()); }
        AuditReport { schema_version: 1, snapshot: delta.snapshot.clone(), records: records.into_values().collect() }
    }
}
/// The paths `changes::between` reported, as a set.
pub(super) fn changed_paths(changes: &[serde_json::Value]) -> BTreeSet<String> {
    changes.iter().filter_map(|c| c["path"].as_str().map(str::to_owned)).collect()
}
