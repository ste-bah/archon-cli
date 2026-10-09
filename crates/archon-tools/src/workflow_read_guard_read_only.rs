//! Issue-58: read-only inspection calls may receive soft reminders to answer.
//!
//! A write-capable call has the read budget, the wall and the thrash cutoff:
//! reading is bounded because writing is what lifts the bound. A read-only
//! call — a planner, critic, verifier or author whose deliverable is its
//! final message — had none of that: live, one author made 129 distinct
//! Read/Grep/Glob calls over 80 minutes and produced nothing, with the host
//! call timeout the only bound. Nothing it read was wrong; it simply never
//! stopped.
//!
//! The soft reminder is counted over the inspection shapes the write-capable
//! path classifies (`Read`, `Grep`, `Glob`, `read-own-evidence` and a Bash
//! command the shell classifier recognises as read-only). It never refuses a
//! call; the runner's no-progress window governs a stalled author. The
//! write-capable guard never reaches this module.
use super::{State, WorkflowReadGuard, inspection_call};
use serde_json::Value;

fn command(input: &Value) -> &str {
    input.get("command").and_then(Value::as_str).unwrap_or("")
}

/// The one-line reminder appended from the configured soft count onward.
pub(super) fn nudge(inspections: u32) -> String {
    format!("You have made {inspections} inspection calls; produce your deliverable now.")
}

/// The read-only verdict for one call, after the shell admissions and the
/// forbidden-path check have passed it. Counts an admitted inspection call;
/// never refuses an inspection call; touches nothing else.
pub(super) fn admit(state: &mut State, name: &str, input: &Value) -> Option<String> {
    if !inspection_call(name, command(input)) {
        return None;
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
        (made >= soft).then(|| nudge(made))
    }

    /// One sentence for the call's opening prompt telling a read-only agent
    /// the soft reminder, so it plans its reading. `None` for a write-capable
    /// guard and for a read-only guard with the reminder disabled, so prompts
    /// are unchanged.
    pub fn preamble(&self) -> Option<String> {
        if !self.read_only() {
            return None;
        }
        let soft = self.read_only_soft_ceiling;
        let shapes = "inspection calls (Read, Grep, Glob and read-only shell commands such as cat, grep, sed -n, git diff)";
        if soft == 0 {
            return None;
        }
        let text = format!(
            "From {soft} {shapes} on, every inspection result reminds you to produce your deliverable; build and test commands are not counted."
        );
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
