//! The host session trace of a structured agent call (Issue 276).
//!
//! A structured call returns only the agent's reply text through the v2
//! client port, so the tool calls its session made were dropped there and
//! the stored result held only what the agent said it read and ran. The
//! dispatch notes each returned session's trace here; the call that opened
//! the scope applies the trace to its parsed result.
use archon_workflow::WorkflowAgentToolUse;
use archon_workflow::v2::tool_trace::merge_session_traces;
use std::cell::RefCell;
use std::future::Future;

tokio::task_local! {
    static SESSIONS: RefCell<Vec<Vec<WorkflowAgentToolUse>>>;
}

/// Run `work` with a fresh capture and return its output with the merged
/// trace of every session that returned inside it (first answer, repair,
/// restarted agent). `None` when no session returned.
pub(in super::super) async fn capture<T>(
    work: impl Future<Output = T>,
) -> (T, Option<Vec<WorkflowAgentToolUse>>) {
    SESSIONS
        .scope(RefCell::new(Vec::new()), async {
            let output = work.await;
            let sessions = SESSIONS.with(|sessions| sessions.take());
            (output, merge_session_traces(sessions))
        })
        .await
}

/// Note one returned session's trace. Outside a capture this does nothing.
pub(in super::super) fn note(tool_uses: &[WorkflowAgentToolUse]) {
    let _ = SESSIONS.try_with(|sessions| sessions.borrow_mut().push(tool_uses.to_vec()));
}
