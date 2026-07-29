use std::path::PathBuf;
use std::sync::Arc;

use archon_core::agent::Agent;
use archon_tui::app::TuiEvent;

use crate::slash_context::SlashCommandContext;

pub(super) async fn drain_pending_export(
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    cmd_ctx: &SlashCommandContext,
    input_tui_tx: &archon_tui::event_channel::TuiEventSender,
) {
    let export_desc = cmd_ctx
        .pending_export_shared
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some(desc) = export_desc {
        let format = desc.format;
        let format_arg = desc.format_arg_display;
        let export_result = {
            let guard = agent.lock().await;
            let messages = &guard.conversation_state().messages;
            archon_session::export::export_session(messages, &cmd_ctx.session_id, format)
        };
        let delivered = match export_result {
            Ok(content) => {
                write_export_content(
                    &cmd_ctx.session_id,
                    format,
                    &format_arg,
                    &content,
                    input_tui_tx,
                )
                .await
            }
            Err(e) => input_tui_tx
                .send_async(TuiEvent::Error(format!("Export failed: {e}")))
                .await
                .is_ok(),
        };
        if !delivered {
            tracing::warn!("export status delivery failed");
            return;
        }
    }
    if let Err(error) = input_tui_tx
        .send_async(TuiEvent::SlashCommandComplete)
        .await
    {
        tracing::warn!(%error, "export command completion delivery failed");
    }
}

async fn write_export_content(
    session_id: &str,
    format: archon_session::export::ExportFormat,
    format_arg: &str,
    content: &str,
    input_tui_tx: &archon_tui::event_channel::TuiEventSender,
) -> bool {
    let export_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("archon")
        .join("exports");
    if let Err(e) = std::fs::create_dir_all(&export_dir) {
        return input_tui_tx
            .send_async(TuiEvent::Error(format!("Failed to create export dir: {e}")))
            .await
            .is_ok();
    }
    let filename = archon_session::export::default_export_filename(session_id, format);
    let path = export_dir.join(&filename);
    match archon_session::export::write_export(content, &path) {
        Ok(()) => {
            let format_arg_display = if format_arg.is_empty() {
                "markdown"
            } else {
                format_arg
            };
            input_tui_tx
                .send_async(TuiEvent::TextDelta(format!(
                    "\nExported ({format_arg_display}) to {}\n",
                    path.display(),
                )))
                .await
                .is_ok()
        }
        Err(e) => input_tui_tx
            .send_async(TuiEvent::Error(format!("Export failed: {e}")))
            .await
            .is_ok(),
    }
}
