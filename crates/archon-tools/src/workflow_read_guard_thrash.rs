//! Issue-54: a coder that keeps calling tools after the read wall, without
//! writing, is stopped instead of left to burn its call budget.
//!
//! Live (wf-caac2ac3, agents-5-0) the coder hit the read wall at call ~55,
//! still reading a 400-line file with `sed -n`, then issued ~1,100 more Bash
//! calls over 70 minutes — refused reads, then `echo uu` / `grep -c "" file`
//! / `echo uv` — with zero files changed. Nothing ended the session; it would
//! have run to the six-hour call budget.
//!
//! Once the budget is exhausted the guard counts every call that is not a
//! substantive write: each refusal (the refused reads first among them) and
//! each allowed Bash call that is not a build or test runner. A build/test
//! command is progress: not counted, but it resets nothing. Only a
//! substantive write — the guard's existing `record_write` verdict, the one
//! that grants reads — lifts the wall and clears the count. Past
//! [`MAX_NON_WRITING_CALLS_AFTER_WALL`] the guard answers with a terminal
//! refusal and repeats it for every later call; the subagent runner reads
//! `terminal_failure` at the end of that tool round and ends the session
//! with the same text, which the write layer treats as a host interruption
//! (partial work captured, the note carried to the next attempt through
//! session memory) rather than a verdict on the task.
use super::{State, shell};
use serde_json::Value;

/// Non-writing calls tolerated after the read budget is exhausted. The call
/// after this many is refused terminally.
pub const MAX_NON_WRITING_CALLS_AFTER_WALL: u32 = 15;

/// The prefix of the terminal refusal and of the session-ending error. The
/// write layer (`archon_workflow`) matches this text to tell the host's cut
/// from a transport failure; the two spellings are pinned by tests on each
/// side.
pub const READ_WALL_THRASH_MARKER: &str = "read-wall thrash:";

pub(super) fn terminal_message(non_writing: u32, writes: u32) -> String {
    format!(
        "{READ_WALL_THRASH_MARKER} {non_writing} non-writing calls after the read budget was exhausted; {writes} substantive write{}",
        if writes == 1 { "" } else { "s" }
    )
}

/// Fold one admitted or refused call into the thrash count. Takes the
/// verdict `admit` reached and returns the verdict to hand back: unchanged
/// until the count passes the cutoff, the terminal refusal from then on.
pub(super) fn observe(
    state: &mut State,
    name: &str,
    input: &Value,
    verdict: Option<String>,
) -> Option<String> {
    if !state.wall_hit {
        return verdict;
    }
    let counted = match &verdict {
        Some(_) => true,
        None => {
            name == "Bash"
                && !shell::build_or_test(input.get("command").and_then(Value::as_str).unwrap_or(""))
        }
    };
    if !counted {
        return verdict;
    }
    state.non_writing_after_wall = state.non_writing_after_wall.saturating_add(1);
    if state.non_writing_after_wall <= MAX_NON_WRITING_CALLS_AFTER_WALL {
        return verdict;
    }
    let terminal = terminal_message(state.non_writing_after_wall, state.writes);
    state.terminal = Some(terminal.clone());
    Some(terminal)
}

/// A substantive write lifts the wall: the allowance is fresh and the
/// count starts over if it is exhausted again.
pub(super) fn on_substantive_write(state: &mut State) {
    state.wall_hit = false;
    state.non_writing_after_wall = 0;
}
