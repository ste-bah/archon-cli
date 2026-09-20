//! Issue-58: a read-only call's inspection is capped, so an agent that cannot
//! write is still made to answer.
//!
//! A write-capable call has the read budget, the wall and the thrash cutoff:
//! reading is bounded because writing is what lifts the bound. A read-only
//! call — a planner, critic, verifier or author whose deliverable is its
//! final message — had none of that: live, one author made 129 distinct
//! Read/Grep/Glob calls over 80 minutes and produced nothing, with the host
//! call timeout the only bound. Nothing it read was wrong; it simply never
//! stopped.
//!
//! Two ceilings, both counted over the inspection shapes the write-capable
//! path classifies (`Read`, `Grep`, `Glob`, `read-own-evidence` and a Bash
//! command the shell classifier recognises as read-only):
//!
//! - at the SOFT ceiling every further inspection call is still admitted,
//!   but its result ends with a one-line nudge naming the count and the hard
//!   ceiling, so the agent can plan its last reads;
//! - at the HARD ceiling further inspection calls are refused with the
//!   instruction to answer from what it has read. The session is NOT ended:
//!   the deliverable is the final message, and refusing reads is exactly
//!   what forces it. Build and test commands are not inspection and are
//!   never refused here.
//!
//! Either ceiling set to 0 is off. The write-capable guard never reaches
//! this module.
use super::{State, WorkflowReadGuard, inspection_call};
use serde_json::Value;

/// The prefix of the hard-ceiling refusal, pinned so a transcript reader or a
/// session-memory consumer can tell it from the write path's budget refusal.
pub const READ_CEILING_MARKER: &str = "read ceiling reached:";

fn command(input: &Value) -> &str {
    input.get("command").and_then(Value::as_str).unwrap_or("")
}

/// The refusal every inspection call gets once the hard ceiling is reached.
/// The same text every time: the count it names is the count of admitted
/// inspection calls, which no longer moves.
pub(super) fn refusal(inspections: u32) -> String {
    format!(
        "{READ_CEILING_MARKER} {inspections} inspection calls; answer now with your deliverable from what you have read. Further Read, Grep, Glob and shell inspection calls are refused; build and test commands still run."
    )
}

/// The one-line nudge appended to an inspection result from the soft ceiling
/// on. Without a hard ceiling there is nothing to warn of past it.
pub(super) fn nudge(inspections: u32, hard: u32) -> String {
    if hard > 0 {
        format!(
            "You have made {inspections} inspection calls; produce your deliverable now — further reading past {hard} will be refused."
        )
    } else {
        format!("You have made {inspections} inspection calls; produce your deliverable now.")
    }
}

/// The read-only verdict for one call, after the shell admissions and the
/// forbidden-path check have passed it. Counts an admitted inspection call;
/// refuses one past the hard ceiling; touches nothing else.
pub(super) fn admit(
    guard: &WorkflowReadGuard,
    state: &mut State,
    name: &str,
    input: &Value,
) -> Option<String> {
    if !inspection_call(name, command(input)) {
        return None;
    }
    let hard = guard.read_only_hard_ceiling;
    if hard > 0 && state.read_only_inspections >= hard {
        return Some(refusal(state.read_only_inspections));
    }
    state.read_only_inspections = state.read_only_inspections.saturating_add(1);
    None
}

impl WorkflowReadGuard {
    /// The text to append to a finished call's result, or `None` to leave
    /// the result alone. Only a read-only guard ever answers, and only for an
    /// inspection call from the soft ceiling on; a write-capable guard's
    /// results are never touched here. Called after `before_tool` admitted
    /// the call and the tool ran, so the count it names includes this call.
    pub fn result_note(&self, name: &str, input: &Value) -> Option<String> {
        if !self.read_only() || !inspection_call(name, command(input)) {
            return None;
        }
        let soft = self.read_only_soft_ceiling;
        if soft == 0 {
            return None;
        }
        let made = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .read_only_inspections;
        (made >= soft).then(|| nudge(made, self.read_only_hard_ceiling))
    }

    /// One sentence for the call's opening prompt telling a read-only agent
    /// the ceilings it runs under, so it plans its reading rather than
    /// discovering the cap at call 120. `None` for a write-capable guard and
    /// for a read-only guard with both ceilings disabled, so those prompts
    /// are unchanged.
    pub fn preamble(&self) -> Option<String> {
        if !self.read_only() {
            return None;
        }
        let soft = self.read_only_soft_ceiling;
        let hard = self.read_only_hard_ceiling;
        let shapes = "inspection calls (Read, Grep, Glob and read-only shell commands such as cat, grep, sed -n, git diff)";
        let text = match (soft, hard) {
            (0, 0) => return None,
            (soft, 0) => format!(
                "Inspection ceiling for this read-only call: from {soft} {shapes} on, every inspection result reminds you to produce your deliverable, so plan your reading to finish before then; build and test commands are not counted."
            ),
            (0, hard) => format!(
                "Inspection ceiling for this read-only call: past {hard} {shapes} further reading is refused and you must answer from what you have read, so plan your reading to finish inside that; build and test commands are not counted."
            ),
            (soft, hard) => format!(
                "Inspection ceiling for this read-only call: from {soft} {shapes} on, every inspection result reminds you to produce your deliverable, and past {hard} such calls further reading is refused and you must answer from what you have read, so plan your reading to finish inside that; build and test commands are not counted."
            ),
        };
        Some(text)
    }

    /// Admitted inspection calls so far; a write-capable guard reports 0.
    pub fn read_only_inspections(&self) -> u32 {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .read_only_inspections
    }
}
