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
//! a new state resets the count. The first revisit escalates (the failing
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
/// (attempts included) reached one already reached.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressLedger {
    pub seen: BTreeSet<Vec<String>>,
    pub revisits: u32,
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

    /// Records `round`; returns the revisits in a row, 0 for a new state.
    pub fn observe(&mut self, round: &AcceptanceRoundRecordV1) -> u32 {
        if self.seen.insert(state_key(round)) {
            self.revisits = 0;
        } else {
            self.revisits = self.revisits.saturating_add(1);
        }
        self.revisits
    }

    fn path(run_dir: &Path) -> std::path::PathBuf {
        run_dir
            .join(ACCEPTANCE_RECORDS_DIR)
            .join(PROGRESS_LEDGER_FILE)
    }

    /// Records are authoritative: the record is written before the ledger,
    /// so even a readable ledger may lag after an interrupted write. Replay
    /// every recorded attempt, including attempts of the resumed round. A
    /// record that cannot be read is an error naming it, never skipped.
    pub fn load(run_dir: &Path, _round: u32) -> crate::WorkflowResult<Self> {
        let history = all_attempts(run_dir)?;
        if !history.is_empty() {
            return Ok(Self::from_history(&history));
        }
        Ok(std::fs::read(Self::path(run_dir))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default())
    }

    pub fn save(&self, run_dir: &Path) -> crate::WorkflowResult<()> {
        let path = Self::path(run_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| crate::WorkflowError::io(parent, e))?;
        }
        let staging = path.with_extension("json.tmp");
        std::fs::write(&staging, serde_json::to_vec_pretty(self)?)
            .map_err(|e| crate::WorkflowError::io(&staging, e))?;
        std::fs::rename(&staging, &path).map_err(|e| crate::WorkflowError::io(&path, e))
    }
}

#[path = "acceptance_record_order.rs"]
mod order;
pub(super) use order::note_recorded;

/// When the record file at `path` was last written, in epoch nanoseconds.
fn file_time(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |at| at.as_nanos())
}

/// Every round record, in the order the records were written (`order`: the
/// log's order, else the record's own file time, never simply first). Gaps
/// in numbering and stale ledgers cannot erase recorded states, and neither
/// can a record that will not read or parse: as for the latest record, that
/// is an error naming the file. Records land whole (staged and renamed), so
/// such a file is damage, never a write in progress.
fn all_attempts(run_dir: &Path) -> crate::WorkflowResult<Vec<AcceptanceRoundRecordV1>> {
    let mut history = Vec::new();
    for round in entries_named(&run_dir.join(ACCEPTANCE_RECORDS_DIR), |name| {
        name.starts_with("round-")
    })? {
        if !round.is_dir() {
            continue;
        }
        let is_record = |name: &str| name.starts_with("attempt-") && name.ends_with(".json");
        for path in entries_named(&round, is_record)? {
            let bytes = std::fs::read(&path).map_err(|e| crate::WorkflowError::io(&path, e))?;
            let record =
                serde_json::from_slice::<AcceptanceRoundRecordV1>(&bytes).map_err(|e| {
                    crate::WorkflowError::StateCorrupt(format!(
                        "acceptance record {} will not parse: {e}",
                        path.display()
                    ))
                })?;
            history.push((file_time(&path), record));
        }
    }
    order::in_recorded_order(run_dir, history)
}

/// The paths in `dir` whose file names `keep` accepts; none when `dir` does
/// not exist (no round recorded yet).
fn entries_named(
    dir: &Path,
    keep: impl Fn(&str) -> bool,
) -> crate::WorkflowResult<Vec<std::path::PathBuf>> {
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(crate::WorkflowError::io(dir, error)),
    };
    let mut paths = Vec::new();
    for entry in listing {
        let entry = entry.map_err(|e| crate::WorkflowError::io(dir, e))?;
        if entry.file_name().to_str().is_some_and(&keep) {
            paths.push(entry.path());
        }
    }
    Ok(paths)
}

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
#[path = "acceptance_progress_tests.rs"]
mod tests;
