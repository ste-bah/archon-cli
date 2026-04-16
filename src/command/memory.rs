//! `/memory` slash command handler.
//! Extracted from main.rs to reduce main.rs from 6234 to < 500 lines.

use std::sync::Arc;

use crate::slash_context::SlashCommandContext;
use archon_memory::MemoryTrait;
use archon_tui::app::TuiEvent;

/// Truncate a string to at most `max` bytes, appending "..." if truncated.
/// Safe for multi-byte UTF-8: always splits on a char boundary.
fn truncate_str(s: &str, max: usize) -> String {
    let trimmed = s.replace('\n', " ");
    if trimmed.len() <= max {
        trimmed
    } else {
        let mut end = max.saturating_sub(3);
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &trimmed[..end])
    }
}

/// Handle `/memory` commands: list, search, clear.
pub async fn handle_memory_command(
    input: &str,
    tui_tx: &tokio::sync::mpsc::Sender<TuiEvent>,
    memory: &Arc<dyn MemoryTrait>,
) {
    let rest = input.strip_prefix("/memory").unwrap_or("").trim();
    let (subcmd, arg) = match rest.split_once(' ') {
        Some((s, a)) => (s.trim(), a.trim()),
        None => (rest, ""),
    };

    match subcmd {
        "" | "list" => match memory.list_recent(10) {
            Ok(memories) if memories.is_empty() => {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta("\nNo memories stored.\n".into()))
                    .await;
            }
            Ok(memories) => {
                let mut out = format!("\nRecent memories ({}):\n", memories.len());
                for m in &memories {
                    let short_id = &m.id[..8.min(m.id.len())];
                    let date = m.created_at.format("%Y-%m-%d %H:%M");
                    out.push_str(&format!(
                        "  [{short_id}] {title} ({mtype}, {date})\n",
                        title = m.title,
                        mtype = m.memory_type,
                    ));
                }
                let _ = tui_tx.send(TuiEvent::TextDelta(out)).await;
            }
            Err(e) => {
                let _ = tui_tx
                    .send(TuiEvent::Error(format!("Memory graph error: {e}")))
                    .await;
            }
        },
        "search" => {
            if arg.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::Error("Usage: /memory search <query>".into()))
                    .await;
                return;
            }
            match memory.recall_memories(arg, 10) {
                Ok(results) if results.is_empty() => {
                    let _ = tui_tx
                        .send(TuiEvent::TextDelta(format!(
                            "\nNo memories matching \"{arg}\".\n"
                        )))
                        .await;
                }
                Ok(results) => {
                    let mut out = format!("\nMemories matching \"{arg}\" ({}):\n", results.len());
                    for m in &results {
                        let short_id = &m.id[..8.min(m.id.len())];
                        out.push_str(&format!(
                            "  [{short_id}] {title} -- {snippet}\n",
                            title = m.title,
                            snippet = truncate_str(&m.content, 80),
                        ));
                    }
                    let _ = tui_tx.send(TuiEvent::TextDelta(out)).await;
                }
                Err(e) => {
                    let _ = tui_tx
                        .send(TuiEvent::Error(format!("Memory search error: {e}")))
                        .await;
                }
            }
        }
        "clear" => match memory.clear_all() {
            Ok(n) => {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!("\nCleared {n} memories from the graph.\n")))
                    .await;
            }
            Err(e) => {
                let _ = tui_tx
                    .send(TuiEvent::Error(format!("Failed to clear memories: {e}")))
                    .await;
            }
        },
        other => {
            let _ = tui_tx
                .send(TuiEvent::Error(format!(
                    "Unknown memory subcommand: {other}. Use list, search, or clear."
                )))
                .await;
        }
    }
}
