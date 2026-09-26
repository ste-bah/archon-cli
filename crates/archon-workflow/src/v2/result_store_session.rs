//! What one run's session recorded or replayed through its result store.
//!
//! Resume replay rules answer only from earlier sessions, and a replayed
//! remediation verdict vouches only for the fix it judged: both need to know
//! what THIS session did, which no record on disk says.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use super::WorkflowV2ResultStore;

fn finish_of(record: &super::WorkflowV2CallRecord) -> &str {
    if record.finished_at.is_empty() {
        &record.started_at
    } else {
        &record.finished_at
    }
}

/// The recorded fix a session's fix was replayed from: its call id, and
/// when the execution whose answer was replayed finished -- proven from
/// that execution's own record, never from whatever record is on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayedFix {
    pub call_id: String,
    pub finished_at: String,
}

#[derive(Debug, Default)]
pub(super) struct SessionLedger {
    calls: Mutex<BTreeSet<String>>,
    /// For each remediation unit and round (`resume_drift::remediation_round_key`):
    /// the recorded fix this session's fix was replayed from, with when the
    /// execution it replayed finished, or `None` when it ran (a fresh agent,
    /// or branches answered from more than one record or unprovably).
    fix_lineage: Mutex<BTreeMap<String, Option<ReplayedFix>>>,
    /// For a round whose fix replayed its own record by re-deriving a
    /// drifted sibling's answer (Issue-109): that sibling, the execution a
    /// re-save of the fix restates.
    refile_origin: Mutex<BTreeMap<String, String>>,
    /// When a record this session overwrote had finished, as the earlier
    /// session left it: re-saving a replayed call must not move its past.
    prior_finish: Mutex<BTreeMap<String, String>>,
    /// Call ids this session wrote a record for (a replay writes none).
    written: Mutex<BTreeSet<String>>,
}

impl WorkflowV2ResultStore {
    /// Mark `call_id` as recorded or replayed in this session.
    pub fn note_session_call(&self, call_id: &str) {
        if let Ok(mut calls) = self.session.calls.lock() {
            calls.insert(call_id.to_string());
        }
    }

    /// Whether this session recorded or replayed `call_id`. A poisoned lock
    /// answers yes: replay rules then fall back to running the call.
    pub fn in_session(&self, call_id: &str) -> bool {
        self.session
            .calls
            .lock()
            .map_or(true, |calls| calls.contains(call_id))
    }

    /// Keep the finish time of the record at `path` the first time this
    /// session overwrites it.
    pub(super) fn note_prior_finish(&self, path: &std::path::Path, call_id: &str) {
        let first_write = self
            .session
            .written
            .lock()
            .map(|mut written| written.insert(call_id.to_string()))
            .unwrap_or(false);
        if !first_write {
            return;
        }
        let Some(record) = std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<super::WorkflowV2CallRecord>(&bytes).ok())
        else {
            return;
        };
        if let Ok(mut prior) = self.session.prior_finish.lock() {
            prior
                .entry(call_id.to_string())
                .or_insert_with(|| finish_of(&record).to_string());
        }
    }

    /// When `record` finished as the earlier sessions recorded it, or `None`
    /// for a record this session wrote first.
    pub fn recorded_finish(&self, record: &super::WorkflowV2CallRecord) -> Option<String> {
        if let Some(prior) = self
            .session
            .prior_finish
            .lock()
            .ok()
            .and_then(|prior| prior.get(&record.call.id).cloned())
        {
            return Some(prior);
        }
        let written = self
            .session
            .written
            .lock()
            .map_or(true, |written| written.contains(&record.call.id));
        (!written).then(|| finish_of(record).to_string())
    }

    /// Record how this session answered the fix of `round_key`: replayed
    /// from a recorded execution, or ran (`None`). The latest answer wins.
    pub fn note_fix_lineage(&self, round_key: &str, source: Option<ReplayedFix>) {
        if let Ok(mut origins) = self.session.refile_origin.lock() {
            origins.remove(round_key);
        }
        if let Ok(mut lineage) = self.session.fix_lineage.lock() {
            lineage.insert(round_key.to_string(), source);
        }
    }

    /// Issue-109: the fix of `round_key`, just noted as replayed from its own
    /// record, answered by re-deriving `sibling`'s outcome. Cleared by the
    /// next [`Self::note_fix_lineage`] of the round.
    pub(crate) fn note_refile_origin(&self, round_key: &str, sibling: &str) {
        if let Ok(mut origins) = self.session.refile_origin.lock() {
            origins.insert(round_key.to_string(), sibling.to_string());
        }
    }

    /// Whether `outcome` is exactly what `persisted` (a branch outcome as
    /// loaded from this store) already records, once saved the way the
    /// store saves it.
    pub(crate) fn filed_unchanged(
        &self,
        persisted: Option<&crate::v2::WorkflowV2BranchOutcome>,
        outcome: &crate::v2::WorkflowV2BranchOutcome,
    ) -> bool {
        persisted.is_some_and(|persisted| {
            super::sanitize_for_persistence(outcome).is_ok_and(|saved| &saved == persisted)
        })
    }

    /// The recorded fix this session's fix of `round_key` was replayed from.
    /// `None` when it ran, was never asked, or the lock is poisoned.
    pub fn fix_replayed(&self, round_key: &str) -> Option<ReplayedFix> {
        self.session
            .fix_lineage
            .lock()
            .ok()
            .and_then(|lineage| lineage.get(round_key).cloned().flatten())
    }

    /// The call id of [`Self::fix_replayed`].
    pub fn fix_replayed_from(&self, round_key: &str) -> Option<String> {
        self.fix_replayed(round_key).map(|fix| fix.call_id)
    }

    /// Issue-111: a remediation fix this session answered by replaying a
    /// recorded execution is re-saved as a NEW record, finishing now. Without
    /// a note of the execution it restates, the next resume took that re-save
    /// for the execution, found it later than the verdict that judged the
    /// real one, and re-asked every verdict of a replayed round. The note is
    /// the replayed record's own, carried forward, or that record itself.
    pub(super) fn stamp_answer_origin(&self, record: &mut super::WorkflowV2CallRecord) {
        use crate::v2::script::resume_verdict::{is_remediation_fix, remediation_round_key};
        if record.answered_by.is_some()
            || record.invalidated_by.is_some()
            || !is_remediation_fix(&record.call)
        {
            return;
        }
        // Only the save that answers this session's call: a NEW attempt (or
        // a first record) under the id. A re-save of the attempt already on
        // disk -- an invalidation, a status rewrite -- restates nothing this
        // session replayed, and stamping it would pin an older execution's
        // finish on a record of a later one.
        let on_disk = std::fs::read(self.result_path(&record.call.id))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<super::WorkflowV2CallRecord>(&bytes).ok());
        if on_disk.is_some_and(|earlier| earlier.attempt >= record.attempt) {
            return;
        }
        let Some(replayed) =
            remediation_round_key(&record.call).and_then(|key| self.fix_replayed(&key))
        else {
            return;
        };
        let source = std::fs::read(self.result_path(&replayed.call_id))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<super::WorkflowV2CallRecord>(&bytes).ok());
        // Issue-109: a fix that replayed its own record by re-deriving a
        // drifted sibling's answer restates the sibling's execution, whose
        // manifest is filed under the sibling's stage; the finish is the
        // one proven for its own record, never the sibling's.
        let refiled = remediation_round_key(&record.call).and_then(|key| {
            self.session
                .refile_origin
                .lock()
                .ok()
                .and_then(|origins| origins.get(&key).cloned())
        });
        record.answered_by = Some(
            source
                .and_then(|source| source.answered_by)
                .filter(|origin| origin.finished_at == replayed.finished_at)
                .unwrap_or(super::WorkflowV2AnswerOrigin {
                    call_id: refiled.unwrap_or(replayed.call_id),
                    finished_at: replayed.finished_at,
                }),
        );
    }

    /// When the execution `record` restates finished: its answer origin
    /// when it has one, else [`Self::recorded_finish`].
    pub fn executed_finish(&self, record: &super::WorkflowV2CallRecord) -> Option<String> {
        match &record.answered_by {
            Some(origin) => Some(origin.finished_at.clone()),
            None => self.recorded_finish(record),
        }
    }
}
