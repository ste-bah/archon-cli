//! In-memory completed messages for explicit workflow validation continuations.
//! A scope is tied to an exact executor id and cannot be inherited by a nested agent.
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct CompletedHistory(Arc<Mutex<Vec<serde_json::Value>>>);

impl CompletedHistory {
    pub fn append(&self, message: &serde_json::Value) {
        // Poisoning is never silently converted into incomplete history.
        self.0
            .lock()
            .expect("completed history poisoned")
            .push(message.clone());
    }

    pub fn messages(&self) -> Vec<serde_json::Value> {
        self.0.lock().expect("completed history poisoned").clone()
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
