//! `TuiEvent` channel-drain handler extracted from `run_inner`.
//!
//! Relocated from `src/event_loop.rs` (L223-L347) per REM-2g (split plan
//! section 3.3, docs/rem-2-split-plan.md). The 30-arm match is kept intact —
//! its exhaustiveness is the correctness contract. The enclosing
//! `#[allow(clippy::cognitive_complexity)]` on `run_inner` (original L176)
//! is replicated here because the match itself drives the complexity score.

use crate::app::{App, McpManager, McpManagerView, SessionPicker, TuiEvent};
use crate::vim::VimState;
use tokio::sync::mpsc::error::TrySendError;

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
        TuiEvent::TransientThinkingDelta(text) => app.on_transient_thinking_delta(&text),
        TuiEvent::CommitThinkingPreview => app.commit_thinking_preview(),
        TuiEvent::DiscardThinkingPreview => app.discard_thinking_preview(),
        TuiEvent::ToolStart { name, id } => app.on_tool_start(&name, &id),
        TuiEvent::ToolOutputChunk { id, chunk } => {
            if let Some(tool) = app.tool_outputs.iter_mut().find(|tool| tool.tool_id == id) {
                tool.append_output(&chunk);
            }
        }
        TuiEvent::ToolComplete {
            name,
            id,
            success,
            output,
            transcript_summary,
        } => {
            app.set_tool_summary(&id, transcript_summary);
            app.on_tool_complete(&name, &id, success, &output);
        }
        TuiEvent::TurnComplete {
            input_tokens,
            output_tokens,
            cache_creation_tokens,
            cache_read_tokens,
        } => {
            super::tui_events_accounting::apply_turn_usage(
                app,
                input_tokens,
                output_tokens,
                cache_creation_tokens,
                cache_read_tokens,
            );
            flush_pending_input_after_turn(app, input_tx);
        }
        TuiEvent::Error(msg) => app.on_error(&msg),
        TuiEvent::GenerationStarted => app.on_generation_started(),
        TuiEvent::SlashCommandComplete => app.on_slash_command_complete(),
        TuiEvent::ThinkingToggle(enabled) => {
            app.show_thinking = enabled;
            let message = if enabled {
                "\nThinking display enabled.\n"
            } else {
                "\nThinking display disabled.\n"
            };
            app.output.append(message);
        }
        TuiEvent::OpenThinkingArchive => app.open_thinking_archive(),
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
        TuiEvent::AskUserPrompt { question, kind } => {
            app.ask_user_prompt = Some(question.clone());
            app.ask_user_prompt_kind = Some(kind);
            app.ask_user_draft.clear();
            app.output.append_line(&format!("[question] {question}"));
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
                // Kept so the `/theme` picker can mark the current entry;
                // `Theme` is a colour struct and cannot be reversed to a name.
                app.theme_name = name;
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
        // Overlay constructors live in `picker_events.rs`; keys for all of
        // them are routed by `picker_input.rs`.
        TuiEvent::ShowMessageSelector(messages) => {
            super::picker_events::open_message_selector(app, messages);
        }
        TuiEvent::ShowSkillsMenu(skills) => {
            super::picker_events::open_skills_menu(app, skills);
        }
        TuiEvent::ShowModelPicker(entries) => {
            super::picker_events::open_model_picker(app, entries);
        }
        TuiEvent::ShowThemePicker(entries) => {
            super::picker_events::open_theme_picker(app, entries);
        }
        TuiEvent::ShowSettings(entries) => {
            super::picker_events::open_settings(app, entries);
        }
        TuiEvent::ShowHooks(entries) => {
            super::picker_events::open_hooks(app, entries);
        }
        TuiEvent::ShowPermissions { mode, rules } => {
            super::picker_events::open_permissions(app, mode, rules);
        }
        TuiEvent::ShowMemoryFiles(entries) => {
            super::picker_events::open_memory_files(app, entries);
        }
        TuiEvent::ShowBranchPicker(entries) => {
            super::picker_events::open_branch_picker(app, entries);
        }
        TuiEvent::ShowVoiceCapture { vad_threshold } => {
            super::picker_events::open_voice_capture(app, vad_threshold);
        }
        TuiEvent::VoiceRecording(recording) => {
            super::picker_events::set_voice_recording(app, recording);
        }
        TuiEvent::VoiceLevel(level) => {
            super::picker_events::push_voice_level(app, level);
        }
        TuiEvent::ShowFilePicker { root, entries } => {
            // TASK-#207 SLASH-FILES: /files opens this overlay; input
            // priority branch (event_loop/input.rs) routes Up/Down,
            // Enter (descend on dir / inject `@<path>` and close on
            // file), Backspace (ascend), Esc (close); render dispatch
            // (render/body.rs draw_file_picker) draws it.
            app.file_picker = Some(crate::screens::file_picker::FilePicker::new(root, entries));
        }
        TuiEvent::ShowSearchResults { query, entries } => {
            // TASK-#208 SLASH-SEARCH: /search opens this overlay; input
            // priority branch routes Up/Down/Enter/Esc. Enter injects
            // `@<absolute-path> ` into the input buffer and closes the
            // overlay (no descend semantics — search results are flat).
            app.search_results = Some(crate::screens::search_results::SearchResults::new(
                query, entries,
            ));
        }
        TuiEvent::OpenView(view_id) => {
            app.open_view(view_id);
            tracing::info!(?view_id, "TuiEvent::OpenView opened view");
        }
        TuiEvent::OpenViewRows { view_id, rows } => {
            let row_count = rows.len();
            app.open_view_with_rows(view_id, rows);
            tracing::info!(?view_id, row_count, "TuiEvent::OpenViewRows opened view");
        }
        TuiEvent::VideoIngestProgress(event) => {
            app.on_video_ingest_progress(event);
        }
        TuiEvent::AgentActivity(update) => {
            app.on_agent_activity(update);
        }
        TuiEvent::ActivityStream(update) => {
            app.on_activity_stream_update(update);
        }
        TuiEvent::ContextPressureUpdated {
            tokens_used,
            context_window,
            cache_creation_tokens,
            cache_read_tokens,
            context_name,
            resolution_source,
            heaviest_message_tokens,
        } => {
            super::tui_events_accounting::apply_context_pressure(
                app,
                tokens_used,
                context_window,
                cache_creation_tokens,
                cache_read_tokens,
                context_name,
                resolution_source,
                heaviest_message_tokens,
            );
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
            // Also show it in the overlay, if one is open. The overlay is
            // centred and the input line is near the bottom, so both are
            // readable at once — and seeing the recognised text next to the
            // levels that produced it is how a bad transcription gets
            // attributed to a quiet microphone rather than to the model.
            if let Some(overlay) = app.voice_capture.as_mut() {
                overlay.set_transcription(&text);
            }
        }
        TuiEvent::SetAgentInfo { name, color } => {
            app.status.agent_name = Some(name);
            app.status.agent_color = color;
        }
        TuiEvent::Resize { cols, rows } => {
            crate::layout::handle_resize(cols, rows);
        }
        TuiEvent::Done => {
            app.should_quit = true;
        }
        // TUI-330: NotificationTimeout was added to events::TuiEvent but
        // never wired through the legacy app::TuiEvent duplicate. After
        // TASK-#246 retired the duplicate (this commit), the match must
        // cover it. The active notification overlay (if any) is dropped
        // on timeout — same effect as Esc on the overlay path.
        TuiEvent::NotificationTimeout(_ms) => {
            // Notification overlays are owned by render::chrome; the
            // event-loop side is a no-op (the timeout simply triggers a
            // re-render which then sees the expiry and clears).
        }
    }
}

fn flush_pending_input_after_turn(app: &mut App, input_tx: &tokio::sync::mpsc::Sender<String>) {
    let mut queued = std::mem::take(&mut app.pending_input).into_iter();
    let mut deferred = Vec::new();

    while let Some(text) = queued.next() {
        match input_tx.try_send(text) {
            Ok(()) => {}
            Err(TrySendError::Full(text)) => {
                deferred.push(text);
                deferred.extend(queued);
                break;
            }
            Err(TrySendError::Closed(_text)) => {
                tracing::warn!("TurnComplete dropped queued input because input channel is closed");
                return;
            }
        }
    }

    if deferred.is_empty() {
        return;
    }

    let count = deferred.len();
    let input_tx = input_tx.clone();
    crate::observability::spawn_named("tui-pending-input-flush", async move {
        for text in deferred {
            if input_tx.send(text).await.is_err() {
                tracing::warn!("TurnComplete deferred input flush stopped because channel closed");
                return;
            }
        }
    });
    tracing::warn!(
        count,
        "TurnComplete deferred queued input flush because input channel was full"
    );
}

#[cfg(test)]
#[path = "tui_events_tests.rs"]
mod tests;
