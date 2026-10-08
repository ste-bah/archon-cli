//! In-memory completed messages for explicit workflow validation continuations.
//! A scope is tied to an exact executor id and cannot be inherited by a nested agent.
use std::sync::{Arc, Mutex};

/// A refusal to continue a completed invocation (#241): it cannot be
/// restored exactly. Carried as this type, never recognised from text, so a
/// workflow can tell it from a failure and start a new agent instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinuationRefused(pub String);

impl std::fmt::Display for ContinuationRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ContinuationRefused {}

impl ContinuationRefused {
    /// The refusal of a client that keeps no completed sessions at all.
    pub fn no_session_kept() -> Self {
        Self(
            "cannot continue agent: this client keeps no completed agent session it can restore \
             exactly; start a new agent"
                .into(),
        )
    }
}

/// The tool name of the summary entry that ends a host session trace
/// (Issue 276). A real tool name cannot hold a `.`, so no tool call can be
/// mistaken for it. Its presence says the session's history was captured; its
/// input counts the calls made, kept and dropped by the trace bound.
pub const TOOL_TRACE_SUMMARY_NAME: &str = "archon.tool_trace";

#[derive(Clone, Default)]
pub struct CompletedHistory(Arc<Mutex<SessionState>>);

#[derive(Default)]
struct SessionState {
    messages: Vec<serde_json::Value>,
    context: Option<RuntimeContext>,
    /// Set by the executor when it refuses to continue this session.
    refusal: Option<String>,
}

#[derive(Clone)]
pub struct RuntimeContext {
    pub system_prompt: String,
    pub model: String,
    pub effort: String,
    pub critical_system_reminder: Option<String>,
}

impl CompletedHistory {
    pub fn append(&self, message: &serde_json::Value) {
        // Poisoning is never silently converted into incomplete history.
        self.0
            .lock()
            .expect("completed history poisoned")
            .messages
            .push(message.clone());
    }

    pub fn context(&self, initial: Option<RuntimeContext>) -> Option<RuntimeContext> {
        let mut state = self.0.lock().expect("completed history poisoned");
        if state.context.is_none() {
            state.context = initial;
        }
        state.context.clone()
    }

    /// Record that continuing this session was refused (#241). Only the
    /// executor calls this, so the host that owns the session can tell a
    /// refusal from any failure, whatever words the failure carries.
    pub fn refuse(&self, why: &str) {
        self.0.lock().expect("completed history poisoned").refusal = Some(why.to_string());
    }

    /// The refusal recorded for this session, taken once.
    pub fn take_refusal(&self) -> Option<String> {
        self.0
            .lock()
            .expect("completed history poisoned")
            .refusal
            .take()
    }

    /// How many messages are held, without copying them.
    pub fn message_count(&self) -> usize {
        self.0
            .lock()
            .expect("completed history poisoned")
            .messages
            .len()
    }

    /// Read the messages held from index `start` on, without copying them.
    /// An out-of-range `start` reads none.
    pub fn read_since<R>(&self, start: usize, read: impl FnOnce(&[serde_json::Value]) -> R) -> R {
        let state = self.0.lock().expect("completed history poisoned");
        read(state.messages.get(start..).unwrap_or_default())
    }

    pub fn messages(&self) -> Vec<serde_json::Value> {
        self.0
            .lock()
            .expect("completed history poisoned")
            .messages
            .clone()
    }
}

#[derive(Clone)]
pub struct SubagentSession {
    pub agent_id: String,
    pub history: CompletedHistory,
    pub continuing: bool,
}

tokio::task_local! { static SESSION: SubagentSession; }

pub fn current_for(agent_id: &str) -> Option<SubagentSession> {
    SESSION
        .try_with(Clone::clone)
        .ok()
        .filter(|s| s.agent_id == agent_id)
}

pub async fn scope<T>(session: SubagentSession, work: impl std::future::Future<Output = T>) -> T {
    SESSION.scope(session, work).await
}

pub async fn inherit<T>(
    session: Option<SubagentSession>,
    work: impl std::future::Future<Output = T>,
) -> T {
    match session {
        Some(session) => scope(session, work).await,
        None => work.await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_scope_is_exact_and_cannot_leak_to_another_agent() {
        let session = SubagentSession {
            agent_id: "one-generation".into(),
            history: CompletedHistory::default(),
            continuing: true,
        };
        scope(session, async {
            assert!(current_for("one-generation").is_some());
            assert!(current_for("other-agent").is_none());
            assert!(current_for("one-generation-new").is_none());
        })
        .await;
        assert!(current_for("one-generation").is_none());
    }
}
