//! `TuiEvent` channel-drain handler extracted from `run_inner`.
//!
//! Relocated from `src/event_loop.rs` (L223-L347) per REM-2g (split plan
//! section 3.3, docs/rem-2-split-plan.md). The 30-arm match is kept intact —
//! its exhaustiveness is the correctness contract. The enclosing
//! `#[allow(clippy::cognitive_complexity)]` on `run_inner` (original L176)
//! is replicated here because the match itself drives the complexity score.

use crate::app::{App, McpManager, McpManagerView, SessionPicker, TuiEvent};
use crate::vim::VimState;

/// Apply a single `TuiEvent` to the running `App`.
///
/// Equivalent to one iteration of the original `while let Ok(tui_event) =
/// event_rx.try_recv()` loop body. Caller is responsible for the outer
/// drain loop and for flushing queued input after `TurnComplete` (the only
/// arm that writes to `input_tx`).
#[allow(clippy::cognitive_complexity)]
pub(super) async fn handle_tui_event(
    app: &mut App,
    event: TuiEvent,
    input_tx: &tokio::sync::mpsc::Sender<String>,
) {
    match event {
        TuiEvent::TextDelta(text) => app.on_text_delta(&text),
        TuiEvent::ThinkingDelta(text) => app.on_thinking_delta(&text),
        TuiEvent::ToolStart { name, id } => app.on_tool_start(&name, &id),
        TuiEvent::ToolComplete {
            name,
            id,
            success,
            output,
        } => {
            app.on_tool_complete(&name, &id, success, &output);
        }
        TuiEvent::TurnComplete {
            input_tokens,
            output_tokens,
        } => {
            app.on_turn_complete();
            // Anthropic pricing: $3/MTok input, $15/MTok output
            app.status.cost +=
                (input_tokens as f64 * 3.0 + output_tokens as f64 * 15.0) / 1_000_000.0;
            // Drain any input queued during generation
            let queued: Vec<String> = app.pending_input.drain(..).collect();
            for text in queued {
                let _ = input_tx.send(text).await;
            }
        }
        TuiEvent::Error(msg) => app.on_error(&msg),
        TuiEvent::GenerationStarted => app.on_generation_started(),
        TuiEvent::SlashCommandComplete => app.on_slash_command_complete(),
        TuiEvent::ThinkingToggle(enabled) => {
            app.show_thinking = enabled;
        }
        TuiEvent::ModelChanged(model) => {
            app.status.model = model;
        }
        TuiEvent::BtwResponse(response) => {
            app.btw_overlay = Some(response);
        }
        TuiEvent::PermissionPrompt {
            tool,
            description: _,
        } => {
            app.permission_prompt = Some(tool);
        }
        TuiEvent::SessionRenamed(name) => {
            app.session_name = Some(name);
        }
        TuiEvent::PermissionModeChanged(mode) => {
            app.status.permission_mode = mode;
        }
        TuiEvent::ShowSessionPicker(sessions) => {
            app.session_picker = Some(SessionPicker {
                sessions,
                selected: 0,
            });
        }
        TuiEvent::SetAccentColor(color) => {
            app.theme.accent = color;
            app.theme.header = color;
            app.theme.border_active = color;
            app.theme.thinking_dot = color;
        }
        TuiEvent::SetTheme(name) => {
            if let Some(t) = crate::theme::theme_by_name(&name) {
                app.theme = t;
            }
        }
        TuiEvent::ShowMcpManager(servers) => {
            app.mcp_manager = Some(McpManager {
                servers,
                view: McpManagerView::ServerList { selected: 0 },
            });
        }
        TuiEvent::UpdateMcpManager(servers) => {
            if let Some(ref mut mgr) = app.mcp_manager {
                mgr.servers = servers;
            }
        }
        TuiEvent::OpenView(view_id) => {
            // TASK-AGS-822: placeholder handler. Full view rendering
            // deferred to Stage 7+ UI tickets. Log the open request
            // so tests and tracing observers can confirm the event
            // landed. Clustered with ShowMcpManager / ShowSessionPicker
            // (other overlay-opening arms) for locality.
            tracing::info!(?view_id, "TuiEvent::OpenView received (placeholder)");
        }
        TuiEvent::SetVimMode(enabled) => {
            if enabled {
                app.vim_state = Some(VimState::new());
            } else {
                app.vim_state = None;
            }
        }
        TuiEvent::VimToggle => {
            if app.vim_state.is_some() {
                app.vim_state = None;
            } else {
                app.vim_state = Some(VimState::new());
            }
        }
        TuiEvent::VoiceText(text) => {
            app.input.inject_text(&text);
        }
        TuiEvent::SetAgentInfo { name, color } => {
            app.status.agent_name = Some(name);
            app.status.agent_color = color;
        }
        TuiEvent::Resize { cols, rows } => {
            crate::layout::handle_resize(cols, rows);
        }
        TuiEvent::UserInput(_) => {
            // TUI-106: handled by run_event_loop; old run_tui path is a no-op.
        }
        TuiEvent::SlashCancel => {
            // TUI-106: handled by run_event_loop; old run_tui path is a no-op.
        }
        TuiEvent::SlashAgent(_) => {
            // TUI-106: handled by run_event_loop; old run_tui path is a no-op.
        }
        TuiEvent::Done => {
            app.should_quit = true;
        }
    }
}
