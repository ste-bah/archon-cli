use std::sync::Arc;

use archon_core::agent::Agent;
use archon_tui::app::TuiEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HistorySendError;

impl std::fmt::Display for HistorySendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TUI event could not be queued")
    }
}

impl std::error::Error for HistorySendError {}

/// Restore `session_id` into the live agent and replay it into the transcript.
///
/// Returns whether the conversation was actually adopted. The caller uses this
/// to decide whether to repoint subsequent writes at `session_id`; repointing
/// after a failed load would write the previous conversation into a session it
/// does not belong to.
pub(super) async fn handle_resume_session(
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    input_tui_tx: &archon_tui::event_channel::TuiEventSender,
    store: &Arc<archon_session::storage::SessionStore>,
    session_id: &str,
) -> bool {
    // Existence is checked separately from message loading because
    // `load_messages` maps a missing session to `Ok(empty)`. Treating that as a
    // successful resume would repoint writes at a session that does not exist,
    // and the next persist would materialise it.
    let exists = match store.get_session(session_id) {
        Ok(meta) => {
            if let Some(name) = meta.name
                && let Err(error) = input_tui_tx
                    .send_async(TuiEvent::SessionRenamed(name))
                    .await
            {
                tracing::warn!(%error, "resumed session name delivery failed");
                return false;
            }
            true
        }
        Err(error) => {
            tracing::warn!(%error, session_id, "resume target session not found");
            false
        }
    };

    let restored = match store.load_messages(session_id) {
        Ok(raw_messages) => {
            let messages = parse_raw_messages(&raw_messages);
            let count = messages.len();
            let banner = format!("\n━━━ Resumed session {session_id} ({count} messages) ━━━\n\n");
            if send_history(input_tui_tx, &banner, &messages).await.is_ok() {
                agent.lock().await.clear_conversation_detached().await;
                agent.lock().await.restore_conversation(messages);
                exists
            } else {
                tracing::error!("failed to replay resumed session history");
                return false;
            }
        }
        Err(e) => {
            if let Err(error) = input_tui_tx
                .send_async(TuiEvent::Error(format!("Failed to load session: {e}")))
                .await
            {
                tracing::warn!(%error, "session load failure delivery failed");
            }
            false
        }
    };
    if let Err(error) = input_tui_tx
        .send_async(TuiEvent::SlashCommandComplete)
        .await
    {
        tracing::warn!(%error, "resume command completion delivery failed");
    }
    restored
}

pub(super) async fn handle_truncate_session(
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    input_tui_tx: &archon_tui::event_channel::TuiEventSender,
    store: &Arc<archon_session::storage::SessionStore>,
    target_session_id: &str,
    idx_str: &str,
) {
    let idx: u64 = match idx_str.trim().parse() {
        Ok(n) => n,
        Err(_) => {
            if let Err(error) = input_tui_tx
                .send_async(TuiEvent::TextDelta(format!(
                    "\n[rewind: invalid index '{idx_str}']\n"
                )))
                .await
            {
                tracing::warn!(%error, "rewind validation delivery failed");
                return;
            }
            if let Err(error) = input_tui_tx
                .send_async(TuiEvent::SlashCommandComplete)
                .await
            {
                tracing::warn!(%error, "rewind command completion delivery failed");
            }
            return;
        }
    };

    if let Err(e) = store.truncate_messages_after(target_session_id, idx) {
        if let Err(error) = input_tui_tx
            .send_async(TuiEvent::Error(format!("Failed to truncate session: {e}")))
            .await
        {
            tracing::warn!(%error, "session truncate failure delivery failed");
            return;
        }
        if let Err(error) = input_tui_tx
            .send_async(TuiEvent::SlashCommandComplete)
            .await
        {
            tracing::warn!(%error, "rewind command completion delivery failed");
        }
        return;
    }

    match store.load_messages(target_session_id) {
        Ok(raw_messages) => {
            let messages = parse_raw_messages(&raw_messages);
            let count = messages.len();
            let banner = format!("\n━━━ Rewound to message {idx} ({count} messages kept) ━━━\n\n");
            if send_history(input_tui_tx, &banner, &messages).await.is_ok() {
                agent.lock().await.clear_conversation_detached().await;
                agent.lock().await.restore_conversation(messages);
            } else {
                tracing::error!("failed to replay rewound session history");
                return;
            }
        }
        Err(e) => {
            if let Err(error) = input_tui_tx
                .send_async(TuiEvent::Error(format!(
                    "Failed to reload session after truncate: {e}"
                )))
                .await
            {
                tracing::warn!(%error, "rewound session reload failure delivery failed");
                return;
            }
        }
    }
    if let Err(error) = input_tui_tx
        .send_async(TuiEvent::SlashCommandComplete)
        .await
    {
        tracing::warn!(%error, "rewind command completion delivery failed");
    }
}

fn parse_raw_messages(raw_messages: &[String]) -> Vec<serde_json::Value> {
    raw_messages
        .iter()
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect()
}

pub(crate) fn history_text(messages: &[serde_json::Value]) -> String {
    let mut history = String::new();
    for msg in messages {
        let content = message_text_content(msg);
        if content.is_empty() {
            continue;
        }
        if msg["role"].as_str() == Some("user") {
            history.push_str("> ");
        }
        history.push_str(&content);
        history.push_str("\n\n");
    }
    history.push_str("━━━ End of history — continue conversation ━━━\n\n");
    history
}

pub(crate) async fn send_history(
    input_tui_tx: &archon_tui::event_channel::TuiEventSender,
    banner: &str,
    messages: &[serde_json::Value],
) -> Result<(), HistorySendError> {
    let mut text = banner.to_string();
    text.push_str(&history_text(messages));
    let event = TuiEvent::TextDelta(text);
    input_tui_tx
        .send_atomic_async(event)
        .await
        .map_err(|_| HistorySendError)
}

fn message_text_content(msg: &serde_json::Value) -> String {
    match &msg["content"] {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|item| item["text"].as_str().map(|s| s.to_string()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    /// Pins the trap that `handle_resume_session`'s separate existence check
    /// exists for.
    ///
    /// `load_messages` maps a missing session to `Ok(empty)` rather than an
    /// error, so "we loaded messages, therefore we resumed" reports success for
    /// an id that was never in the store. Adopting that id as the write target
    /// would make the next `replace_messages` materialise the row. If this
    /// assertion ever flips to an `Err`, the existence check can collapse back
    /// into the load.
    #[test]
    fn load_messages_reports_empty_rather_than_error_for_a_missing_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = archon_session::storage::SessionStore::open(&dir.path().join("sessions.db"))
            .expect("open session store");

        let loaded = store
            .load_messages("no-such-session-id")
            .expect("a missing session must not surface as an error");

        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn display_history_waits_for_capacity() {
        let (tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
        tx.send(archon_tui::app::TuiEvent::Done)
            .expect("fill event channel");
        let delivery =
            tokio::spawn(async move { super::send_history(&tx, "history\n", &[]).await });
        tokio::task::yield_now().await;

        assert!(
            !delivery.is_finished(),
            "full queue must backpressure history"
        );
        assert!(matches!(
            rx.recv().await,
            Some(archon_tui::app::TuiEvent::Done)
        ));
        delivery
            .await
            .expect("history task")
            .expect("history delivery");
        assert!(matches!(
            rx.recv().await,
            Some(archon_tui::app::TuiEvent::TextDelta(text)) if text.starts_with("history")
        ));
    }

    #[tokio::test]
    async fn display_history_waits_for_atomic_multiframe_capacity() {
        let (tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(2);
        tx.send(archon_tui::app::TuiEvent::Done)
            .expect("fill one queue slot");
        let messages = vec![serde_json::json!({
            "role": "assistant",
            "content": "x".repeat(archon_tui::event_channel::MAX_COALESCED_CONTENT_BYTES + 1)
        })];
        let delivery = tokio::spawn(async move { super::send_history(&tx, "", &messages).await });
        tokio::task::yield_now().await;

        assert!(
            !delivery.is_finished(),
            "multi-frame history must wait for capacity for the whole batch"
        );
        assert!(matches!(
            rx.recv().await,
            Some(archon_tui::app::TuiEvent::Done)
        ));
        delivery
            .await
            .expect("history task")
            .expect("history delivery");

        let first = rx.recv().await.expect("first history frame");
        let second = rx.recv().await.expect("second history frame");
        assert!(matches!(first, archon_tui::app::TuiEvent::TextDelta(_)));
        assert!(matches!(second, archon_tui::app::TuiEvent::TextDelta(_)));
    }
}
