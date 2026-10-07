//! The acceptance loop's budget follows progress, never a round count (A2,
//! Issue 262).
//!
//! Progress is reaching a failing state this run never reached, where the
//! state is the set of failing check ids (never the failure text: output
//! that differs by a hash or a temp name would read as new every round).
//! Fewer failures than some earlier round is not the measure: after a
//! regression, every round that repairs one check more reaches a new state
//! and is progress, though it fails more than the best round before it.
//! A round that reaches a state already reached counts toward the stall;
//! a new state resets the count. Concurrent identical observations from the
//! same reserved recording frontier count once: remediation had no chance
//! to happen between them. Legacy records with no frontier retain their
//! original counting rule. The first revisit escalates (the failing
//! checks go to every owner together); the [`ACCEPTANCE_STALL_LIMIT`]th in
//! a row PAUSES the run with its evidence: it never ends the loop and never
//! fails the run. No round count ends or pauses it. The states reached and
//! the count are kept in [`ProgressLedger`], persisted beside the round
//! records, so a pause and resume (a round's next attempt) keeps them.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

// Mounted under `acceptance_stage`: `super` is that module.
use super::{
    ACCEPTANCE_RECORDS_DIR, AcceptanceRoundRecordV1, attempt_file_name, next_attempt, round_dir,
};

/// Revisits in a row that pause the run: the no-progress bound. The first
/// of them escalates instead.
pub const ACCEPTANCE_STALL_LIMIT: u32 = 2;

/// Why the loop pauses the run: no progress for the stall limit.
pub const PAUSE_NO_PROGRESS: &str = "no_progress";

/// The ledger's file, under the acceptance records directory.
pub const PROGRESS_LEDGER_FILE: &str = "progress-ledger.json";

/// What the host tells the script about the loop after a round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopDecision {
    /// The loop ends with this round.
    pub final_round: bool,
    /// No progress since an earlier round: remediation goes to every owner
    /// of the failing checks as one cross-owner unit.
    pub escalate: bool,
    /// Trailing rounds, this one included, that made no progress.
    pub stalled_rounds: u32,
    /// The run pauses after this round, for this cause
    /// ([`PAUSE_NO_PROGRESS`]); the loop is not over.
    pub pause: Option<&'static str>,
}

/// A round's failing state: the sorted set of its failing check ids.
pub fn state_key(record: &AcceptanceRoundRecordV1) -> Vec<String> {
    let ids: BTreeSet<String> = (record.failing_checks().into_iter())
        .map(|check| check.check_id.clone())
        .collect();
    ids.into_iter().collect()
}

/// The failing states a run reached, and how many rounds in a row
/// (attempts included) reached one already reached. `observed` is the
/// state of each record the ledger saw, in order: a second copy of what the
/// records hold, from which a damaged record's state is rebuilt.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressLedger {
    pub seen: BTreeSet<Vec<String>>,
    pub revisits: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed: Vec<ObservedState>,
}

/// One record's failing state, as the ledger observed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedState {
    pub round: u32,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_frontier: Option<u64>,
    pub state: Vec<String>,
}

impl ProgressLedger {
    /// The ledger `history` (oldest first) leaves.
    pub fn from_history(history: &[AcceptanceRoundRecordV1]) -> Self {
        let mut ledger = Self::default();
        for round in history {
            ledger.observe(round);
        }
        ledger
    }

    /// The ledger the observed states (oldest first) leave.
    pub fn from_states(states: impl IntoIterator<Item = ObservedState>) -> Self {
        let mut ledger = Self::default();
        for state in states {
            ledger.observe_state(state);
        }
        ledger
    }

    #[cfg(test)]
    pub fn observe_at(
        &mut self,
        run_dir: &Path,
        round: &AcceptanceRoundRecordV1,
    ) -> crate::WorkflowResult<u32> {
        let mut round = round.clone();
        round.progress_frontier =
            super::reservation::frontier(run_dir, round.round, round.attempt)?;
        Ok(self.observe(&round))
    }

    /// Records `round`; returns the revisits in a row, 0 for a new state.
    pub fn observe(&mut self, round: &AcceptanceRoundRecordV1) -> u32 {
        self.observe_state(ObservedState {
            round: round.round,
            attempt: round.attempt,
            progress_frontier: round.progress_frontier,
            state: state_key(round),
        })
    }

    fn observe_state(&mut self, observed: ObservedState) -> u32 {
        let simultaneous_duplicate = observed.progress_frontier.is_some()
            && self.observed.iter().any(|earlier| {
                earlier.progress_frontier == observed.progress_frontier
                    && earlier.state == observed.state
            });
        if simultaneous_duplicate {
            // Overlapping observations are one opportunity for remediation,
            // not sequential failed attempts at making progress.
        } else if self.seen.insert(observed.state.clone()) {
            self.revisits = 0;
        } else {
            self.revisits = self.revisits.saturating_add(1);
        }
        self.observed.push(observed);
        self.revisits
    }

    /// The state this ledger observed for (`round`, `attempt`).
    fn state_of(&self, round: u32, attempt: u32) -> Option<&Vec<String>> {
        (self.observed.iter().rev())
            .find(|observed| (observed.round, observed.attempt) == (round, attempt))
            .map(|observed| &observed.state)
    }

    fn path(run_dir: &Path) -> std::path::PathBuf {
        run_dir
            .join(ACCEPTANCE_RECORDS_DIR)
            .join(PROGRESS_LEDGER_FILE)
    }

    /// The saved copy: absence is distinct from damage or an I/O fault.
    fn saved(run_dir: &Path) -> crate::WorkflowResult<Option<Self>> {
        let path = Self::path(run_dir);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(crate::WorkflowError::io(&path, error)),
        };
        serde_json::from_slice(&bytes).map(Some).map_err(|error| {
            crate::WorkflowError::StateCorrupt(format!(
                "acceptance progress ledger {} will not parse: {error}",
                path.display()
            ))
        })
    }

    /// Records are authoritative: the record is written before the ledger,
    /// so even a readable ledger may lag after an interrupted write. Replay
    /// every recorded attempt, including attempts of the resumed round. A
    /// damaged record is quarantined ([`Self::load_healing`]); an I/O error
    /// is returned, for the caller to pause on.
    pub fn load(run_dir: &Path, _round: u32) -> crate::WorkflowResult<Self> {
        Self::load_healing(run_dir).map(|healed| healed.ledger)
    }

    /// Saved whole (staged, synced, renamed) and its directory synced: it
    /// is the copy a damaged record's state is rebuilt from.
    pub fn save(&self, run_dir: &Path) -> crate::WorkflowResult<()> {
        let path = Self::path(run_dir);
        let staging = path.with_extension("json.tmp");
        crate::store::write_atomic(&staging, &path, &serde_json::to_vec_pretty(self)?)?;
        match path.parent() {
            Some(dir) => super::sync_record_dirs(run_dir, dir),
            None => Ok(()),
        }
    }
}

#[path = "acceptance_progress_heal.rs"]
mod heal;
pub use heal::{
    HealedLedger, QUARANTINE_DIR, QuarantinedRecordV1, acknowledge_quarantined,
    record_quarantine_events, record_quarantine_events_owned,
};
pub(super) use heal::{highest_quarantined_attempt, quarantined_attempt};

#[path = "acceptance_record_order.rs"]
mod order;
pub(super) use order::{frontier_locked, note_recorded_locked, under_order_lock};

/// Whether `current` reached a failing state none of `history` reached.
pub fn made_progress(
    history: &[AcceptanceRoundRecordV1],
    current: &AcceptanceRoundRecordV1,
) -> bool {
    let now = state_key(current);
    !history.iter().any(|earlier| state_key(earlier) == now)
}

/// Revisits in a row, ending with `current`.
pub fn stalled_rounds(
    history: &[AcceptanceRoundRecordV1],
    current: &AcceptanceRoundRecordV1,
) -> u32 {
    ProgressLedger::from_history(history).observe(current)
}

/// Whether the loop ends after `current`, given the earlier rounds.
pub fn decide(
    history: &[AcceptanceRoundRecordV1],
    current: &AcceptanceRoundRecordV1,
) -> LoopDecision {
    decide_with(&mut ProgressLedger::from_history(history), current)
}

/// Whether the loop ends after `current`, recording it in `ledger`.
///
/// A clean round ends it. Otherwise it goes on while there is anything to
/// act on -- a failed check a task is named to fix, or a failure the host
/// repairs itself (an error, a contract defect, a round-level error). The
/// first revisit escalates and the [`ACCEPTANCE_STALL_LIMIT`]th in a row
/// pauses the run. A round with nothing to act on ends it.
pub fn decide_with(ledger: &mut ProgressLedger, current: &AcceptanceRoundRecordV1) -> LoopDecision {
    if !current.blocks_completion() {
        // Observed all the same, so the ledger's copy covers every record.
        ledger.observe(current);
        return LoopDecision {
            final_round: true,
            escalate: false,
            stalled_rounds: 0,
            pause: None,
        };
    }
    let stalled = ledger.observe(current);
    let actionable = current.has_remediable_failures() || !current.operational_errors.is_empty();
    LoopDecision {
        final_round: !actionable,
        escalate: actionable && (1..ACCEPTANCE_STALL_LIMIT).contains(&stalled),
        stalled_rounds: stalled,
        pause: (actionable && stalled >= ACCEPTANCE_STALL_LIMIT).then_some(PAUSE_NO_PROGRESS),
    }
}

/// The latest attempt of every round before `round`, oldest first: what the
/// loop has recorded so far this execution. A round with no readable record
/// is skipped.
pub fn earlier_rounds(run_dir: &Path, round: u32) -> Vec<AcceptanceRoundRecordV1> {
    (1..round)
        .filter_map(|earlier| {
            let attempt = next_attempt(run_dir, earlier).checked_sub(1)?;
            let path = round_dir(run_dir, earlier).join(attempt_file_name(attempt));
            let bytes = std::fs::read(path).ok()?;
            serde_json::from_slice(&bytes).ok()
        })
        .collect()
}

#[cfg(test)]
#[path = "acceptance_progress_heal_evidence_tests.rs"]
mod heal_evidence_tests;
#[cfg(test)]
#[path = "acceptance_progress_heal_tests.rs"]
mod heal_tests;
#[cfg(test)]
#[path = "acceptance_progress_tests.rs"]
pub(super) mod tests;
