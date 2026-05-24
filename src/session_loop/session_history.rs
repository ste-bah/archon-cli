use std::sync::Arc;

use archon_core::agent::Agent;
use archon_tui::app::TuiEvent;

pub(super) async fn handle_resume_session(
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    input_tui_tx: &archon_tui::event_channel::TuiEventSender,
    store: &Arc<archon_session::storage::SessionStore>,
    session_id: &str,
) {
    if let Ok(meta) = store.get_session(session_id)
        && let Some(name) = meta.name
    {
        let _ = input_tui_tx.send(TuiEvent::SessionRenamed(name));
    }

    match store.load_messages(session_id) {
        Ok(raw_messages) => {
            let messages = parse_raw_messages(&raw_messages);
            let count = messages.len();
            agent.lock().await.clear_conversation_detached().await;
            let _ = input_tui_tx.send(TuiEvent::TextDelta(format!(
                "\n━━━ Resumed session {session_id} ({count} messages) ━━━\n\n"
            )));
            display_history(input_tui_tx, &messages);
            agent.lock().await.restore_conversation(messages);
        }
        Err(e) => {
            let _ = input_tui_tx.send(TuiEvent::Error(format!("Failed to load session: {e}")));
        }
    }
    let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
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
            let _ = input_tui_tx.send(TuiEvent::TextDelta(format!(
                "\n[rewind: invalid index '{idx_str}']\n"
            )));
            let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
            return;
        }
    };

    if let Err(e) = store.truncate_messages_after(target_session_id, idx) {
        let _ = input_tui_tx.send(TuiEvent::Error(format!("Failed to truncate session: {e}")));
        let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
        return;
    }

    match store.load_messages(target_session_id) {
        Ok(raw_messages) => {
            let messages = parse_raw_messages(&raw_messages);
            let count = messages.len();
            agent.lock().await.clear_conversation_detached().await;
            let _ = input_tui_tx.send(TuiEvent::TextDelta(format!(
                "\n━━━ Rewound to message {idx} ({count} messages kept) ━━━\n\n"
            )));
            display_history(input_tui_tx, &messages);
            agent.lock().await.restore_conversation(messages);
        }
        Err(e) => {
            let _ = input_tui_tx.send(TuiEvent::Error(format!(
                "Failed to reload session after truncate: {e}"
            )));
        }
    }
    let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
}

fn parse_raw_messages(raw_messages: &[String]) -> Vec<serde_json::Value> {
    raw_messages
        .iter()
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect()
}

fn display_history(
    input_tui_tx: &archon_tui::event_channel::TuiEventSender,
    messages: &[serde_json::Value],
) {
    for msg in messages {
        let content = message_text_content(msg);
        if content.is_empty() {
            continue;
        }
        let label = match msg["role"].as_str().unwrap_or("unknown") {
            "user" => "> ",
            "assistant" => "",
            _ => "",
        };
        let _ = input_tui_tx.send(TuiEvent::TextDelta(format!("{label}{content}\n\n")));
    }
    let _ = input_tui_tx.send(TuiEvent::TextDelta(
        "━━━ End of history — continue conversation ━━━\n\n".to_string(),
    ));
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
