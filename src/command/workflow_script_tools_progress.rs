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
//! - Since the last other host call (an agent, a checkpoint), a tool call
//!   that brings no NEW answer is counted: its answer was already seen in
//!   this window, whatever its tool and arguments. After
//!   [`REPEAT_STALL_CALLS`] such calls with no new answer between them, the
//!   next tool call is refused BEFORE it runs and the run pauses with the
//!   window as evidence.
//!
//! Round 2: the first version counted only back-to-back identical calls. A
//! cursor loop A, B, A, B, or one that bumped an argument while the answer
//! stayed the same, reset that streak on every call, and the JS watchdog
//! restarts on every host call, so such a run stayed Running forever with no
//! evidence. Counting answers already seen catches both, while a long run of
//! distinct answers (ten thousand distinct files) never pauses: each one is
//! new. A new answer resets the count; another host call resets the window,
//! because the world the calls read may have changed.
//!
//! The window lives in memory for one execution. A resumed run replays its
//! recorded calls and re-runs its tool calls from the start, so the window is
//! rebuilt from the same calls rather than carried over from a stale one.

use std::collections::{HashSet, VecDeque};

use sha2::{Digest, Sha256};

use super::RunToolResponse;

/// Most bytes one tool result may carry back to a script.
///
/// The old run total, now per call: every single result a script could get
/// before is still whole.
pub(crate) const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

/// Tool calls in a row that brought no new answer and make a stall.
///
/// A loop with no model in it reaches this in well under a second. It is
/// set high so that a script scanning many inputs that legitimately answer
/// alike (an empty file, a search with no match) keeps a wide margin: a
/// single new answer anywhere in the run of calls resets the count.
pub(crate) const REPEAT_STALL_CALLS: usize = 1_000;

/// Recent calls kept for the evidence of a stall.
const RECENT_CALLS: usize = 6;

/// One recent tool call, as the evidence shows it.
#[derive(Debug, Clone, serde::Serialize)]
struct RecentCall {
    call_id: String,
    tool: String,
    arguments: String,
    answer_sha256: String,
    new_answer: bool,
}

/// What one script run's tool calls have done since the last other host
/// call, for the stall rule.
///
/// `calls` and `bytes` are diagnostics for the stall evidence; nothing is
/// refused on them.
#[derive(Debug, Default)]
pub(crate) struct ToolCallBudget {
    pub calls: usize,
    pub bytes: usize,
    /// Digests of every answer seen in this window.
    answers: HashSet<[u8; 32]>,
    /// Calls since the last one that brought a new answer.
    without_new: usize,
    last_new_call_id: Option<String>,
    recent: VecDeque<RecentCall>,
    stall: Option<serde_json::Value>,
}

fn answer_digest(response: &RunToolResponse) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([u8::from(response.is_error)]);
    hasher.update(response.content.as_bytes());
    hasher.finalize().into()
}

fn preview(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

impl ToolCallBudget {
    /// Refuse the next tool call once the window has gone
    /// [`REPEAT_STALL_CALLS`] calls without a new answer, before it runs.
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
        if self.without_new < REPEAT_STALL_CALLS {
            return None;
        }
        self.stall = Some(serde_json::json!({
            "event": "script_tool_stall_pause",
            "cause": "no_progress",
            "tool": name,
            "arguments": preview(arguments, 2_000),
            "calls_without_a_new_answer": self.without_new,
            "distinct_answers_in_window": self.answers.len(),
            "last_new_answer_call_id": self.last_new_call_id,
            "recent_calls": self.recent,
            "refused_call_id": call_id,
            "refused_call_executed": false,
            "tool_calls_this_execution": self.calls,
        }));
        Some(format!(
            "workflow script tool calls brought no new answer for {} calls in a row; \
             the next call ({name}, {call_id}) was not run and the run is paused for no progress",
            self.without_new
        ))
    }

    /// Record a call that ran.
    pub(crate) fn record(
        &mut self,
        call_id: &str,
        name: &str,
        arguments: &str,
        response: &RunToolResponse,
    ) {
        self.calls = self.calls.saturating_add(1);
        self.bytes = self.bytes.saturating_add(response.content.len());
        let digest = answer_digest(response);
        let new_answer = self.answers.insert(digest);
        if new_answer {
            self.without_new = 0;
            self.last_new_call_id = Some(call_id.to_string());
        } else {
            self.without_new += 1;
        }
        if self.recent.len() == RECENT_CALLS {
            self.recent.pop_front();
        }
        self.recent.push_back(RecentCall {
            call_id: call_id.to_string(),
            tool: name.to_string(),
            arguments: preview(arguments, 300),
            answer_sha256: hex::encode(digest),
            new_answer,
        });
    }

    /// Another host call happened: the next tool call starts a new window.
    pub(crate) fn break_streak(&mut self) {
        self.answers.clear();
        self.without_new = 0;
        self.last_new_call_id = None;
        self.recent.clear();
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
