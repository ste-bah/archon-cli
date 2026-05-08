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
            flush_pending_input_after_turn(app, input_tx);
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
        TuiEvent::ShowMessageSelector(messages) => {
            // TASK-TUI-620 + followup: /rewind opens this overlay; input
            // priority branch (event_loop/input.rs) routes Up/Down/Enter/Esc
            // and render dispatch (render/body.rs draw_message_selector)
            // draws it.
            app.message_selector = Some(crate::screens::message_selector::MessageSelector::new(
                messages,
            ));
        }
        TuiEvent::ShowSkillsMenu(skills) => {
            // TASK-TUI-627 + followup: /skills opens this overlay; input
            // priority branch (event_loop/input.rs) routes Up/Down/Enter/Esc
            // and render dispatch (render/body.rs draw_skills_menu) draws
            // it. Enter injects `/{skill-name} ` into the input buffer.
            app.skills_menu = Some(crate::screens::skills_menu::SkillsMenu::new(skills));
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
        TuiEvent::AgentActivity(update) => {
            app.on_agent_activity(update);
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
mod tests {
    use super::*;
    use serial_test::serial;
    use std::time::Duration;

    // Both tests below clear and read the process-global task registry
    // (archon_observability::reset_task_registry_for_tests + task_snapshots).
    // task_registry.rs documents this race: parallel tests wipe each other's
    // entries mid-flight. Marking them #[serial(task_registry)] matches the
    // pattern the registry's own tests use (task_registry.rs:163,178,189).
    // Surfaced by CI run 25541207525 on commit bee8d8b under cargo llvm-cov,
    // where instrumentation widens the race window enough to flip pass→fail.
    #[tokio::test]
    #[serial(task_registry)]
    async fn turn_complete_flushes_pending_input_without_blocking_when_channel_has_room() {
        archon_observability::reset_task_registry_for_tests();
        let mut app = App::new();
        app.pending_input.push("first".to_string());
        app.pending_input.push("second".to_string());
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);

        handle_tui_event(
            &mut app,
            TuiEvent::TurnComplete {
                input_tokens: 1,
                output_tokens: 1,
            },
            &tx,
        )
        .await;

        assert!(app.pending_input.is_empty());
        assert_eq!(rx.try_recv().unwrap(), "first");
        assert_eq!(rx.try_recv().unwrap(), "second");
    }

    #[tokio::test]
    #[serial(task_registry)]
    async fn turn_complete_does_not_block_when_input_channel_is_full() {
        archon_observability::reset_task_registry_for_tests();
        let mut app = App::new();
        app.pending_input.push("queued".to_string());
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        tx.try_send("occupied".to_string()).unwrap();

        tokio::time::timeout(
            Duration::from_millis(50),
            handle_tui_event(
                &mut app,
                TuiEvent::TurnComplete {
                    input_tokens: 1,
                    output_tokens: 1,
                },
                &tx,
            ),
        )
        .await
        .expect("TurnComplete handler must not await on a full input channel");

        assert!(app.pending_input.is_empty());
        assert_eq!(rx.try_recv().unwrap(), "occupied");
        assert!(
            archon_observability::task_snapshots()
                .iter()
                .any(|task| task.name == "tui-pending-input-flush")
        );
        archon_observability::abort_alive_tasks();
    }
}
