//! Issue 299: a script's tool calls are limited on NO-PROGRESS only.
//!
//! There used to be two run totals here, 500 calls and 8 MiB of output, and
//! crossing either failed the run as `SpecInvalid`. Both measured how much a
//! script did, not whether it was getting anywhere: a script that read 501
//! distinct files failed, and the call that crossed the line had already run
//! when its result was refused.
//!
//! What remains bounds each call and detects a stall:
//!
//! - One result is cut at [`MAX_RESULT_BYTES`] with a mark that says so. The
//!   call has run once and its effect stands; the script is told it saw part
//!   of the answer, never handed a silently short one.
//! - A call repeated with identical arguments that got an identical answer
//!   [`REPEAT_STALL_CALLS`] times in a row is a loop that learns nothing. The
//!   next identical call is refused BEFORE it runs and the run pauses with the
//!   streak as evidence. Any other host call (an agent, a checkpoint) breaks
//!   the streak, because the world the repeated call reads may have changed.
//!
//! The streak lives in memory for one execution. A resumed run replays its
//! recorded calls and re-runs its tool calls from the start, so the streak is
//! rebuilt from the same calls rather than carried over from a stale one.

use super::RunToolResponse;

/// Most bytes one tool result may carry back to a script.
///
/// The old run total, now per call: every single result a script could get
/// before is still whole.
pub(crate) const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

/// Identical calls with identical answers, in a row, that make a stall.
///
/// A loop with no model in it repeats a call in microseconds, so a stall
/// reaches this almost at once, while a script that re-reads one file a few
/// times between other work never comes near it.
pub(crate) const REPEAT_STALL_CALLS: usize = 100;

/// The run of identical calls with identical answers that ended last.
#[derive(Debug)]
struct RepeatStreak {
    name: String,
    arguments: String,
    content: String,
    is_error: bool,
    count: usize,
    first_call_id: String,
    last_call_id: String,
}

/// What one script run's tool calls have done, for the stall rule.
///
/// `calls` and `bytes` are diagnostics for the stall evidence; nothing is
/// refused on them.
#[derive(Debug, Default)]
pub(crate) struct ToolCallBudget {
    pub calls: usize,
    pub bytes: usize,
    streak: Option<RepeatStreak>,
    stall: Option<serde_json::Value>,
}

impl ToolCallBudget {
    /// Refuse a call that would extend a full streak, before it runs.
    ///
    /// Returns the pause message and keeps the evidence for
    /// [`Self::take_stall`]. The refused call is not executed, so nothing is
    /// run and then discarded.
    pub(crate) fn refuse_repeat(
        &mut self,
        call_id: &str,
        name: &str,
        arguments: &str,
    ) -> Option<String> {
        let streak = self.streak.as_ref()?;
        if streak.count < REPEAT_STALL_CALLS || streak.name != name || streak.arguments != arguments
        {
            return None;
        }
        let preview: String = streak.content.chars().take(500).collect();
        self.stall = Some(serde_json::json!({
            "event": "script_tool_stall_pause",
            "cause": "no_progress",
            "tool": name,
            "arguments": arguments.chars().take(2_000).collect::<String>(),
            "identical_calls_in_a_row": streak.count,
            "first_call_id": streak.first_call_id,
            "last_call_id": streak.last_call_id,
            "refused_call_id": call_id,
            "refused_call_executed": false,
            "answer_is_error": streak.is_error,
            "answer_bytes": streak.content.len(),
            "answer_preview": preview,
            "tool_calls_this_execution": self.calls,
        }));
        Some(format!(
            "workflow script tool {name} was called {} times in a row with identical arguments \
             and got an identical answer each time; call {call_id} was not run and the run is \
             paused for no progress",
            streak.count
        ))
    }

    /// Record a call that ran, extending or replacing the streak.
    pub(crate) fn record(
        &mut self,
        call_id: &str,
        name: &str,
        arguments: &str,
        response: &RunToolResponse,
    ) {
        self.calls = self.calls.saturating_add(1);
        self.bytes = self.bytes.saturating_add(response.content.len());
        if let Some(streak) = self.streak.as_mut()
            && streak.name == name
            && streak.arguments == arguments
            && streak.is_error == response.is_error
            && streak.content == response.content
        {
            streak.count += 1;
            streak.last_call_id = call_id.to_string();
            return;
        }
        self.streak = Some(RepeatStreak {
            name: name.to_string(),
            arguments: arguments.to_string(),
            content: response.content.clone(),
            is_error: response.is_error,
            count: 1,
            first_call_id: call_id.to_string(),
            last_call_id: call_id.to_string(),
        });
    }

    /// Another host call happened: the next tool call starts a new streak.
    pub(crate) fn break_streak(&mut self) {
        self.streak = None;
    }

    /// The evidence of the stall [`Self::refuse_repeat`] last reported.
    pub(crate) fn take_stall(&mut self) -> Option<serde_json::Value> {
        self.stall.take()
    }
}

/// Cut one oversized result to [`MAX_RESULT_BYTES`], marked, on a character
/// boundary. A result within the bound is returned unchanged.
pub(crate) fn bound_response(mut response: RunToolResponse) -> RunToolResponse {
    let original = response.content.len();
    if original <= MAX_RESULT_BYTES {
        return response;
    }
    let mut cut = MAX_RESULT_BYTES;
    while !response.content.is_char_boundary(cut) {
        cut -= 1;
    }
    response.content.truncate(cut);
    response.content.push_str(&format!(
        "\n[archon: tool result truncated: {cut} of {original} bytes shown; the call ran once \
         and was not repeated]"
    ));
    response.truncated = true;
    response.original_bytes = Some(original);
    response
}
