//! The acceptance loop's budget follows progress, never a round count (A2).
//!
//! A fixed three-round ceiling ended the loop while checks were still
//! getting fixed one at a time: the run then sat unfinished until a person
//! resumed it. Here the host decides from its own records whether the loop
//! ends. A round made progress when it fails fewer checks (plus round-level
//! errors) than any earlier round did, or when its failing state -- which
//! checks fail with which status, and how many round errors, never the
//! output text -- is one no earlier round was in. The first round without
//! progress escalates (the failing checks go to every owner together); a
//! second consecutive one is a stall, and a stall PAUSES the run with its
//! evidence (Issue 262): it never ends the loop and never fails the run, so
//! an operator can act and resume. No round count ends the loop either:
//! [`ACCEPTANCE_RUNAWAY_GUARD`] counts rounds since the last real progress
//! (a round failing fewer checks than every round before it) and pauses,
//! never ends, a loop that keeps moving to new failing states without ever
//! failing fewer; one round of real progress resets it.

use std::collections::BTreeSet;
use std::path::Path;

// Mounted under `acceptance_stage`: `super` is that module.
use super::{AcceptanceRoundRecordV1, attempt_file_name, next_attempt, round_dir};

/// Consecutive rounds without progress that pause the run: the
/// no-progress bound. The first of them escalates instead.
pub const ACCEPTANCE_STALL_LIMIT: u32 = 2;

/// Rounds since the last real progress (fewer failures than every earlier
/// round) after which the run pauses. Never a total: a round of real
/// progress resets it, so a loop that keeps shrinking its failing set runs
/// as long as it needs, and a resumed round that shrinks it goes on. It
/// stops only a loop that keeps moving to failing states it never saw
/// without ever failing fewer, which the stall limit cannot see.
pub const ACCEPTANCE_RUNAWAY_GUARD: usize = 64;

/// Why the loop pauses the run: no progress for the stall limit.
pub const PAUSE_NO_PROGRESS: &str = "no_progress";
/// Why the loop pauses the run: the runaway guard.
pub const PAUSE_RUNAWAY_GUARD: &str = "runaway_guard";

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
    /// ([`PAUSE_NO_PROGRESS`], [`PAUSE_RUNAWAY_GUARD`]); the loop is not over.
    pub pause: Option<&'static str>,
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

/// Rounds after the last one that failed fewer checks than every round
/// before it, `current` included; 0 when `current` is that round.
pub fn rounds_since_real_progress(
    history: &[AcceptanceRoundRecordV1],
    current: &AcceptanceRoundRecordV1,
) -> usize {
    let mut fewest = usize::MAX;
    let mut since = 0;
    for round in history.iter().chain(std::iter::once(current)) {
        let failed = failures(round);
        if failed < fewest {
            fewest = failed;
            since = 0;
        } else {
            since += 1;
        }
    }
    since
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
/// the [`ACCEPTANCE_STALL_LIMIT`]th consecutive one pauses the run, as does
/// [`ACCEPTANCE_RUNAWAY_GUARD`]. A round with nothing to act on ends it.
pub fn decide(
    history: &[AcceptanceRoundRecordV1],
    current: &AcceptanceRoundRecordV1,
) -> LoopDecision {
    if !current.blocks_completion() {
        return LoopDecision {
            final_round: true,
            escalate: false,
            stalled_rounds: 0,
            pause: None,
        };
    }
    let stalled = stalled_rounds(history, current);
    let actionable = current.has_remediable_failures() || !current.operational_errors.is_empty();
    let pause = if !actionable {
        None
    } else if stalled >= ACCEPTANCE_STALL_LIMIT {
        Some(PAUSE_NO_PROGRESS)
    } else if rounds_since_real_progress(history, current) >= ACCEPTANCE_RUNAWAY_GUARD {
        Some(PAUSE_RUNAWAY_GUARD)
    } else {
        None
    };
    LoopDecision {
        final_round: !actionable,
        escalate: actionable && (1..ACCEPTANCE_STALL_LIMIT).contains(&stalled),
        stalled_rounds: stalled,
        pause,
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
