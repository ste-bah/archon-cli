//! The acceptance loop's budget follows progress, never a round count (A2).
//!
//! A fixed three-round ceiling ended the loop while checks were still
//! getting fixed one at a time: the run then sat unfinished until a person
//! resumed it. Here the host decides from its own records whether the loop
//! ends. A round made progress when it fails fewer checks (plus round-level
//! errors) than any earlier round did, or when its failing state -- which
//! checks fail with which status, and how many round errors, never the
//! output text -- is one no earlier round was in. The first round without
//! progress escalates (the failing checks go to every owner together); only
//! a second consecutive one ends the loop, and a round that ends it this way
//! still blocks completion. Behind that, [`ACCEPTANCE_ROUND_CEILING`] ends
//! the loop however it moves, still blocking: the states are finite, and so
//! is the loop.

use std::collections::BTreeSet;
use std::path::Path;

// Mounted under `acceptance_stage`: `super` is that module.
use super::{AcceptanceRoundRecordV1, attempt_file_name, next_attempt, round_dir};

/// Consecutive rounds without progress that end the loop. The first of
/// them escalates instead.
pub const ACCEPTANCE_STALL_LIMIT: u32 = 2;

/// Rounds one execution may run however it progresses; reaching it ends
/// the loop with the gate still blocking, never passing.
pub const ACCEPTANCE_ROUND_CEILING: usize = 12;

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
}

/// A round's failing state: which checks fail and with what status, and
/// how many round-level errors it had. Deliberately NOT the failure text:
/// output that differs by a hash, a temp name or an assertion message would
/// read as a new state every round and fund the loop for ever.
fn state(record: &AcceptanceRoundRecordV1) -> (BTreeSet<(String, String)>, usize) {
    let failing = (record.failing_checks().into_iter())
        .map(|check| (check.check_id.clone(), format!("{:?}", check.status)))
        .collect();
    (failing, record.operational_errors.len())
}

fn failures(record: &AcceptanceRoundRecordV1) -> usize {
    record.failing_checks().len() + record.operational_errors.len()
}

/// Whether `current` made progress over `history` (earlier rounds, oldest
/// first): fewer failures than any of them, or a failing state none of
/// them was in. The first round always has.
pub fn made_progress(
    history: &[AcceptanceRoundRecordV1],
    current: &AcceptanceRoundRecordV1,
) -> bool {
    let Some(fewest) = history.iter().map(failures).min() else {
        return true;
    };
    if failures(current) < fewest {
        return true;
    }
    let now = state(current);
    !history.iter().any(|earlier| state(earlier) == now)
}

/// Trailing rounds, ending with `current`, that made no progress.
pub fn stalled_rounds(
    history: &[AcceptanceRoundRecordV1],
    current: &AcceptanceRoundRecordV1,
) -> u32 {
    let mut stalled = 0;
    let mut end = history.len();
    let mut round = current;
    loop {
        if made_progress(&history[..end], round) {
            return stalled;
        }
        stalled += 1;
        let Some(previous) = end.checked_sub(1) else {
            return stalled;
        };
        end = previous;
        round = &history[end];
    }
}

/// Whether the loop ends after `current`, given the earlier rounds.
///
/// A clean round ends it. Otherwise it goes on while there is anything to
/// act on -- a failed check a task is named to fix, or a failure the host
/// repairs itself (an error, a contract defect, a round-level error) -- and
/// the rounds keep making progress; the first stalled round escalates and
/// the [`ACCEPTANCE_STALL_LIMIT`]th consecutive one ends it.
pub fn decide(
    history: &[AcceptanceRoundRecordV1],
    current: &AcceptanceRoundRecordV1,
) -> LoopDecision {
    if !current.blocks_completion() {
        return LoopDecision {
            final_round: true,
            escalate: false,
            stalled_rounds: 0,
        };
    }
    let stalled = stalled_rounds(history, current);
    let actionable = current.has_remediable_failures() || !current.operational_errors.is_empty();
    // A hard ceiling behind the progress rule: the loop always ends (the
    // gate still blocks), however the failing set moves.
    let ceiling = history.len() + 1 >= ACCEPTANCE_ROUND_CEILING;
    LoopDecision {
        final_round: !actionable || stalled >= ACCEPTANCE_STALL_LIMIT || ceiling,
        escalate: actionable && stalled >= 1 && stalled < ACCEPTANCE_STALL_LIMIT,
        stalled_rounds: stalled,
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
