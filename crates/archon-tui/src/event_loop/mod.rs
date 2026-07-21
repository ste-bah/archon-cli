//! Live render and terminal-input loop for the TUI.
//!
//! `run_inner` is the production event loop. It receives rendered-state
//! events through [`tui_events::handle_tui_event`] and terminal input through
//! [`input::handle_key_event`]. Prompt submission, cancellation, and slash
//! routing are owned by `src/session_loop`.

use std::io;

use anyhow::Result;
use crossterm::event::EventStream;
use ratatui::Terminal;

use crate::app::{App, AppConfig};

use driver::{
    IDLE_TICK_CADENCE, LoopEvent, TickScheduler, animation_cadence, drain_tui_events,
    next_loop_event,
};

mod ask_user;
mod driver;
mod input;
mod mouse;
pub(crate) mod thinking_archive;
mod tui_events;

/// Backend-generic event loop body (TUI-310 extraction from `app.rs`).
///
/// The public generic [`crate::app::run_with_backend`] entry retains live
/// terminal input by calling [`run_inner`].
///
/// **No terminal lifecycle here**: this helper assumes raw mode / alternate
/// screen / mouse capture have already been arranged (or are not needed, for
/// `TestBackend`). Both callers handle their own setup and teardown.
///
/// REM-2g: per-iteration work (TuiEvent drain + keyboard dispatch) is
/// delegated to `tui_events::handle_tui_event` and `input::handle_key_event`.
/// Behavior is equivalent to the pre-split single-file version — helpers
/// mutate `&mut app` in place and communicate back through `app` state only.
#[allow(clippy::cognitive_complexity)]
pub(crate) async fn run_inner<B>(
    config: AppConfig,
    terminal: &mut Terminal<B>,
) -> Result<(), io::Error>
where
    B: ratatui::backend::Backend,
{
    run_inner_with_terminal_events(config, terminal, Some(EventStream::new())).await
}

/// Backend-generic loop body with optional terminal input.
///
/// `run_with_backend_without_terminal_events` passes `None`; generic callers
/// use [`run_inner`] with an [`EventStream`].
pub(crate) async fn run_inner_without_terminal_events<B>(
    config: AppConfig,
    terminal: &mut Terminal<B>,
) -> Result<(), io::Error>
where
    B: ratatui::backend::Backend,
{
    run_inner_with_terminal_events(config, terminal, None).await
}

async fn run_inner_with_terminal_events<B>(
    config: AppConfig,
    terminal: &mut Terminal<B>,
    mut terminal_events: Option<EventStream>,
) -> Result<(), io::Error>
where
    B: ratatui::backend::Backend,
{
    let AppConfig {
        mut event_rx,
        input_tx,
        model,
        splash,
        btw_tx,
        permission_tx,
        ask_user_tx,
        context_window,
        context_source,
        context_threshold,
        command_catalog,
    } = config;

    crate::commands::set_catalog(command_catalog);

    let mut app = App::new();
    app.status.model = model;
    app.status.context_window = context_window;
    app.status.context_name = Some("main".to_string());
    app.status.resolution_source = context_source;
    app.status.compact_threshold = context_threshold;
    match splash {
        Some(cfg) => {
            app.splash_model = cfg.model;
            app.splash_working_dir = cfg.working_dir;
            app.splash_activity = cfg.activity;
        }
        // `splash: None` is the bare-mode / headless-test contract: no
        // welcome screen, start directly on the empty output buffer so the
        // first agent event (or scripted TextDelta) is rendered on the next
        // frame. Matches how `session.rs` constructs `splash_opt` when the
        // user passes `--bare`.
        None => {
            app.show_splash = false;
        }
    }

    let keymap = crate::keybindings::KeyMap::default();
    let mut tick_scheduler = TickScheduler::new(IDLE_TICK_CADENCE);

    loop {
        terminal.draw(|frame| crate::render::draw(frame, &mut app))?;
        tick_scheduler.reconfigure(animation_cadence(&app));

        match next_loop_event(terminal_events.as_mut(), &mut event_rx, &mut tick_scheduler).await {
            LoopEvent::Terminal(event) => {
                input::handle_key_event(
                    &mut app,
                    event,
                    &input_tx,
                    btw_tx.as_ref(),
                    permission_tx.as_ref(),
                    ask_user_tx.as_ref(),
                    &keymap,
                )
                .await;
            }
            LoopEvent::TerminalStreamError(error) => {
                // Non-TTY backends cannot provide crossterm input. Match the
                // previous poll-error behavior by continuing with TUI events
                // and animation ticks instead of failing the render loop.
                tracing::warn!(error = %error, "terminal event stream unavailable; disabling input stream");
                terminal_events = None;
            }
            LoopEvent::TerminalStreamClosed => {
                tracing::warn!("terminal event stream closed; disabling input stream");
                terminal_events = None;
            }
            LoopEvent::Tui(tui_event) => {
                drain_tui_events(&mut app, *tui_event, &mut event_rx, &input_tx).await;
            }
            LoopEvent::TuiChannelClosed => break,
            LoopEvent::Tick => {
                app.input.ultrathink.tick();
                app.thinking.tick_thinking();
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

/// Return the action strings available for a given server entry.
///
/// The order is significant — it's the display order in the menu.
pub(crate) fn mcp_actions_for(server: &crate::app::McpServerEntry) -> Vec<&'static str> {
    let mut actions: Vec<&'static str> = Vec::new();
    if server.disabled {
        actions.push("enable");
    } else {
        if matches!(server.state.as_str(), "crashed" | "stopped") {
            actions.push("reconnect");
        }
        if server.state == "ready" {
            actions.push("tools");
        }
        actions.push("disable");
    }
    actions.push("back");
    actions
}

/// Return the number of actions for a server (used for Down key wrap).
pub(crate) fn mcp_action_count(server: Option<&crate::app::McpServerEntry>) -> usize {
    match server {
        Some(s) => mcp_actions_for(s).len(),
        None => 1, // just "back"
    }
}
