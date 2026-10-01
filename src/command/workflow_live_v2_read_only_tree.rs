//! REM-16 (Major 2): the tripwire around a review branch that was granted
//! a shell, over every root the run's records name.
//!
//! The OS boundary refuses a granted reviewer's writes; this is the defence
//! in depth behind it. Before the map's first pass the host records the
//! watch set (`review_roots`: the checkout with its ignored files, the
//! project root, and every artifact, data, deliverable, acceptance-input and
//! declared-file directory the run's records name, wherever it lives) by
//! (type, size, mtime), spilling to the run's own scratch the bytes of every
//! file git cannot give back. Each granted branch registers when it starts;
//! when it ends the watch set is walked again. A change is put back -- a new
//! path removed, a changed or removed one restored from git or from its
//! spilled bytes -- and EVERY branch in flight when it was found fails: a
//! change cannot be attributed among concurrent branches, so none of them
//! is trusted. A failed branch has no verdict; the map's re-run loop asks it
//! again, and one that keeps changing the tree stays unreviewed. A moved
//! HEAD is never rewound; it fails the branches and is named. The spilled
//! bytes are removed with the tripwire.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;

#[path = "workflow_live_v2_read_only_tree_state.rs"]
mod state;
use state::{Restore, Snapshot, changed, head_note, put_back, snapshot};
pub(super) use state::{WatchSet, git};

pub(super) struct ReviewTreeTripwire {
    watch: WatchSet,
    baseline: Snapshot,
    restore: Restore,
    in_flight: Mutex<BTreeSet<String>>,
    tainted: Mutex<BTreeMap<String, String>>,
}

impl ReviewTreeTripwire {
    /// Record `watch`, spilling what git cannot restore under `spill`;
    /// `None` when nothing is watched or the bytes cannot be kept.
    pub(super) fn arm(watch: WatchSet, spill: &Path) -> Option<Self> {
        if watch.roots.is_empty() {
            return None;
        }
        let baseline = snapshot(&watch);
        let restore = Restore::spill(&watch, &baseline, spill)?;
        Some(Self {
            watch,
            baseline,
            restore,
            in_flight: Mutex::new(BTreeSet::new()),
            tainted: Mutex::new(BTreeMap::new()),
        })
    }

    pub(super) fn enter(&self, branch: &str) {
        self.in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(branch.to_string());
    }

    /// The branch ended: put back any change and fail every branch in
    /// flight when it was found. `Err` names what changed.
    pub(super) fn leave(&self, branch: &str) -> Result<(), String> {
        let mut in_flight = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        let now = snapshot(&self.watch);
        let paths = changed(&self.baseline, &now, &self.restore);
        let mut notes: Vec<String> = head_note(&self.baseline, &now).into_iter().collect();
        notes.extend(put_back(&self.baseline, &paths, &self.restore));
        let mut tainted = self.tainted.lock().unwrap_or_else(|e| e.into_inner());
        if !notes.is_empty() {
            let found = notes.join("; ");
            for id in in_flight.iter() {
                tainted.insert(id.clone(), found.clone());
            }
        }
        in_flight.remove(branch);
        match tainted.remove(branch) {
            Some(found) => Err(format!(
                "review branch {branch}: a watched root changed while it ran, so its verdict is not trusted; the host reverted the change: {found}"
            )),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
#[path = "workflow_live_v2_read_only_tree_tests.rs"]
mod tests;
