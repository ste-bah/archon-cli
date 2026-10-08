//! The host session trace of an agent call (Issue 276).
//!
//! A structured call returns only the agent's reply text through the v2
//! client port, so the tool calls its session made were dropped there and
//! the stored result held only what the agent said it read and ran. The
//! dispatch notes each returned session's trace here, and notes every
//! session whose calls can never reach it (a failed attempt a transient
//! retry replaced, a session that ended in an error); the call that opened
//! the scope applies both to its result.
use archon_workflow::WorkflowAgentToolUse;
use archon_workflow::v2::tool_trace::merge_session_traces;
use std::cell::RefCell;
use std::future::Future;

#[derive(Default)]
struct Capture {
    sessions: Vec<Vec<WorkflowAgentToolUse>>,
    lost: Vec<String>,
}

tokio::task_local! {
    static CAPTURE: RefCell<Capture>;
}

/// What a capture saw: the merged trace of every session that returned
/// (`None` when none did), and why calls are missing from it.
pub(in super::super) struct Captured {
    pub(in super::super) trace: Option<Vec<WorkflowAgentToolUse>>,
    pub(in super::super) lost: Vec<String>,
}

/// Run `work` with a fresh capture and return its output with what the
/// capture saw (first answer, repairs, restarted agent).
pub(in super::super) async fn capture<T>(work: impl Future<Output = T>) -> (T, Captured) {
    CAPTURE
        .scope(RefCell::new(Capture::default()), async {
            let output = work.await;
            let capture = CAPTURE.with(|capture| capture.take());
            let captured = Captured {
                trace: merge_session_traces(capture.sessions),
                lost: capture.lost,
            };
            (output, captured)
        })
        .await
}

/// Note one returned session's trace. Outside a capture this does nothing.
pub(in super::super) fn note(tool_uses: &[WorkflowAgentToolUse]) {
    let _ = CAPTURE.try_with(|capture| capture.borrow_mut().sessions.push(tool_uses.to_vec()));
}

/// Note that a session's tool calls will never reach the trace, and why.
pub(in super::super) fn note_lost(reason: &str) {
    let _ = CAPTURE.try_with(|capture| capture.borrow_mut().lost.push(reason.to_string()));
}

/// Why a transient retry leaves calls out of the trace.
pub(in super::super) const RETRIED: &str = "a transient provider retry replaced a failed attempt; that attempt's tool calls are not in \
     the trace";
/// Why a failed session leaves calls out of the trace.
pub(in super::super) const SESSION_FAILED: &str =
    "a session of this call ended in an error; its tool calls are not in the trace";

/// Whether a dispatch's transient retry replaced an attempt. Set from the
/// retry callback, read once the provider work returns, so the note lands
/// in the dispatching task's capture wherever the attempts ran.
#[derive(Default)]
pub(in super::super) struct Retried(std::sync::atomic::AtomicBool);

impl Retried {
    pub(in super::super) fn mark(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Note the calls this dispatch lost: a replaced attempt's, and, when
    /// the dispatch itself `failed`, its session's.
    pub(in super::super) fn note(&self, failed: bool) {
        if self.0.load(std::sync::atomic::Ordering::Relaxed) {
            note_lost(RETRIED);
        }
        if failed {
            note_lost(SESSION_FAILED);
        }
    }
}
