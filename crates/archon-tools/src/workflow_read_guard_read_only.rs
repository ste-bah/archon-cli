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
use super::{AgentKey, State, WorkflowReadGuard, inspection_call};
use serde_json::Value;

/// Tool names whose calls are withdrawn once a read-only call reaches its
/// hard inspection ceiling.
pub const READ_ONLY_INSPECTION_TOOLS: &[&str] = &["Read", "Grep", "Glob", "read-own-evidence"];

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
        "{READ_CEILING_MARKER} {inspections} inspection calls; answer now with your deliverable from what you have read. {ANSWER_IN_TASK_FORMAT} Further Read, Grep, Glob and shell inspection calls are refused; build and test commands still run."
    )
}

/// What every "answer now" instruction adds: the answer is the task's own
/// deliverable in the task's own format. Live, a body author told only to
/// "answer from what you have read" returned a report about its reading in
/// place of the file its task required, and the attempt was lost.
pub const ANSWER_IN_TASK_FORMAT: &str = "Your answer is the deliverable your task specifies, in exactly the format the task specifies: when the task says your reply must be a file or a document, reply with that file or document and nothing else. Do not reply with a report about your reading, the inspection limit or what you could not check; where something is unverified, apply the task's own rule for it.";

/// The refusal every inspection call gets once inspection was withdrawn for
/// repeated results (Issue-396).
pub(super) fn withdrawn_refusal(reason: &str) -> String {
    format!(
        "{INSPECTION_WITHDRAWN_MARKER} {reason} Answer now with your deliverable from what you have read. {ANSWER_IN_TASK_FORMAT} Further Read, Grep, Glob and shell inspection calls are refused; build and test commands still run."
    )
}

/// The prefix of the refusal after a withdrawal for repeated results, kept
/// apart from [`READ_CEILING_MARKER`]: no count was reached.
pub const INSPECTION_WITHDRAWN_MARKER: &str = "inspection withdrawn:";

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
    key: &AgentKey,
    name: &str,
    input: &Value,
) -> Option<String> {
    if !inspection_call(name, command(input)) {
        return None;
    }
    let agent = state.agent_state.entry(key.clone()).or_default();
    if let Some(reason) = &agent.inspection_withdrawn {
        return Some(withdrawn_refusal(reason));
    }
    let hard = guard.read_only_hard_ceiling;
    if hard > 0 && agent.read_only_inspections >= hard {
        return Some(refusal(agent.read_only_inspections));
    }
    agent.read_only_inspections = agent.read_only_inspections.saturating_add(1);
    None
}

/// Return the admission charge when the cached answer omits already-visible bytes.
pub(super) fn refund_inspection(state: &mut State, key: &AgentKey) {
    if let Some(agent) = state.agent_state.get_mut(key) {
        agent.read_only_inspections = agent.read_only_inspections.saturating_sub(1);
    }
}

impl WorkflowReadGuard {
    /// Whether this read-only call has exhausted its configured inspection
    /// allowance. Write-capable guards and a disabled hard ceiling never
    /// report reached.
    #[cfg(test)]
    pub fn read_only_ceiling_reached(&self) -> bool {
        self.read_only_ceiling_reached_for_key(&(String::new(), None))
    }

    pub fn read_only_ceiling_reached_for(&self, ctx: &crate::tool::ToolContext) -> bool {
        self.read_only_ceiling_reached_for_key(&(ctx.session_id.clone(), ctx.subagent_id.clone()))
    }

    fn read_only_ceiling_reached_for_key(&self, key: &AgentKey) -> bool {
        if !self.read_only() || self.read_only_hard_ceiling == 0 {
            return false;
        }
        self.read_only_inspections_for_key(key) >= self.read_only_hard_ceiling
    }

    /// Whether this read-only call may no longer inspect: its hard ceiling
    /// is reached, or inspection was withdrawn (Issue-396). The runner then
    /// withdraws the inspection tools and asks for the deliverable.
    #[cfg(test)]
    pub fn inspection_withdrawn(&self) -> bool {
        self.inspection_withdrawn_for_key(&(String::new(), None))
    }

    pub fn inspection_withdrawn_for(&self, ctx: &crate::tool::ToolContext) -> bool {
        self.inspection_withdrawn_for_key(&(ctx.session_id.clone(), ctx.subagent_id.clone()))
    }

    fn inspection_withdrawn_for_key(&self, key: &AgentKey) -> bool {
        self.read_only_ceiling_reached_for_key(key) || self.withdrawal_reason_for_key(key).is_some()
    }

    /// Why inspection was withdrawn before the hard ceiling, if it was.
    #[cfg(test)]
    pub fn withdrawal_reason(&self) -> Option<String> {
        self.withdrawal_reason_for_key(&(String::new(), None))
    }

    pub fn withdrawal_reason_for(&self, ctx: &crate::tool::ToolContext) -> Option<String> {
        self.withdrawal_reason_for_key(&(ctx.session_id.clone(), ctx.subagent_id.clone()))
    }

    fn withdrawal_reason_for_key(&self, key: &AgentKey) -> Option<String> {
        if !self.read_only() {
            return None;
        }
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .agent_state
            .get(key)
            .and_then(|agent| agent.inspection_withdrawn.clone())
    }

    /// Withdraw inspection from a read-only call that keeps getting the same
    /// results (Issue-396): its deliverable is its final message, so it is
    /// asked for it rather than failed. `true` only when this call withdrew
    /// it; a write-capable guard, or a call already withdrawn (by this or
    /// by its hard ceiling), returns `false` and the caller keeps its stop.
    #[cfg(test)]
    pub fn withdraw_inspection(&self, reason: &str) -> bool {
        self.withdraw_inspection_for_key(&(String::new(), None), reason)
    }

    pub fn withdraw_inspection_for(&self, ctx: &crate::tool::ToolContext, reason: &str) -> bool {
        self.withdraw_inspection_for_key(&(ctx.session_id.clone(), ctx.subagent_id.clone()), reason)
    }

    fn withdraw_inspection_for_key(&self, key: &AgentKey, reason: &str) -> bool {
        if !self.read_only() || self.inspection_withdrawn_for_key(key) {
            return false;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .agent_state
            .entry(key.clone())
            .or_default()
            .inspection_withdrawn = Some(reason.to_string());
        true
    }

    /// Tool names the hard ceiling refuses independent of input. Bash is
    /// intentionally excluded because its inspection classification depends
    /// on the command supplied for that call.
    pub fn is_read_only_inspection_tool(name: &str) -> bool {
        READ_ONLY_INSPECTION_TOOLS.contains(&name)
    }

    /// Admitted inspection calls so far; a write-capable guard reports 0.
    #[cfg(test)]
    pub fn read_only_inspections(&self) -> u32 {
        self.read_only_inspections_for_key(&(String::new(), None))
    }

    pub fn read_only_inspections_for(&self, ctx: &crate::tool::ToolContext) -> u32 {
        self.read_only_inspections_for_key(&(ctx.session_id.clone(), ctx.subagent_id.clone()))
    }

    fn read_only_inspections_for_key(&self, key: &AgentKey) -> u32 {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .agent_state
            .get(key)
            .map_or(0, |agent| agent.read_only_inspections)
    }

    /// The text to append to a finished call's result, or `None` to leave
    /// the result alone. Only a read-only guard ever answers, and only for an
    /// inspection call from the soft ceiling on; a write-capable guard's
    /// results are never touched here. Called after `before_tool` admitted
    /// the call and the tool ran, so the count it names includes this call.
    #[cfg(test)]
    pub fn result_note(&self, name: &str, input: &Value) -> Option<String> {
        self.result_note_for_key(&(String::new(), None), name, input)
    }

    pub fn result_note_for(
        &self,
        ctx: &crate::tool::ToolContext,
        name: &str,
        input: &Value,
    ) -> Option<String> {
        self.result_note_for_key(
            &(ctx.session_id.clone(), ctx.subagent_id.clone()),
            name,
            input,
        )
    }

    fn result_note_for_key(&self, key: &AgentKey, name: &str, input: &Value) -> Option<String> {
        if !self.read_only() || !inspection_call(name, command(input)) {
            return None;
        }
        let soft = self.read_only_soft_ceiling;
        if soft == 0 {
            return None;
        }
        let made = self.read_only_inspections_for_key(key);
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
        if soft == 0 && hard == 0 {
            return None;
        }
        let shapes = "inspection calls (Read, Grep, Glob and read-only shell commands such as cat, grep, sed -n, git diff)";
        let text = match (soft, hard) {
            (0, hard) => format!(
                "Inspection ceiling for this read-only call: past {hard} {shapes} further reading is refused and you must answer from what you have read, so plan your reading to finish inside that; build and test commands are not counted."
            ),
            (soft, 0) => format!(
                "Inspection ceiling for this read-only call: from {soft} {shapes} on, every inspection result reminds you to produce your deliverable; build and test commands are not counted."
            ),
            (soft, hard) => format!(
                "Inspection ceiling for this read-only call: from {soft} {shapes} on, every inspection result reminds you to produce your deliverable, and past {hard} such calls further reading is refused and you must answer from what you have read, so plan your reading to finish inside that; build and test commands are not counted."
            ),
        };
        Some(text)
    }
}
