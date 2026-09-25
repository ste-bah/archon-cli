//! What one run's session recorded or replayed through its result store.
//!
//! Resume replay rules answer only from earlier sessions, and a replayed
//! remediation verdict vouches only for the fix it judged: both need to know
//! what THIS session did, which no record on disk says.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use super::WorkflowV2ResultStore;

#[derive(Debug, Default)]
pub(super) struct SessionLedger {
    calls: Mutex<BTreeSet<String>>,
    /// For each remediation unit and round (`resume_drift::remediation_round_key`):
    /// the recorded fix this session's fix was replayed from, or `None` when
    /// it ran (a fresh agent, or branches answered from more than one record).
    fix_lineage: Mutex<BTreeMap<String, Option<String>>>,
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

    /// Record how this session answered the fix of `round_key`: replayed
    /// from `source`, or ran (`None`). The latest answer wins.
    pub fn note_fix_lineage(&self, round_key: &str, source: Option<String>) {
        if let Ok(mut lineage) = self.session.fix_lineage.lock() {
            lineage.insert(round_key.to_string(), source);
        }
    }

    /// The recorded fix this session's fix of `round_key` was replayed from.
    /// `None` when it ran, was never asked, or the lock is poisoned.
    pub fn fix_replayed_from(&self, round_key: &str) -> Option<String> {
        self.session
            .fix_lineage
            .lock()
            .ok()
            .and_then(|lineage| lineage.get(round_key).cloned().flatten())
    }
}
