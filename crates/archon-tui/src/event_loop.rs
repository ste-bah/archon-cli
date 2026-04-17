//! Non-blocking event loop for the TUI (TUI-106).
//!
//! `run_event_loop` is the entry point that TASK-TUI-107 will wire into
//! main.rs. It consumes TuiEvents from an unbounded channel, drives
//! [`AgentDispatcher`] (spawn/cancel/switch/poll), and polls completion
//! on a 16ms interval so finished turns drain within one frame.
//!
//! ## Spec Deviation (inherited from TUI-100)
//!
//! Spec references `Arc<dyn Agent>` and `Arc<dyn AgentRouter>`. Neither
//! trait exists: `archon_core::agent::Agent` is a concrete struct, not
//! a trait. Resolution carried forward from TUI-100: [`EventLoopConfig`]
//! takes `Arc<dyn TurnRunner>` (defined in `task_dispatch.rs`) for the
//! agent-execution seat and `Arc<dyn AgentRouter>` (also in
//! `task_dispatch.rs`) for the agent-switching seat. The bridge from
//! the concrete `archon_core` `Agent` to `TurnRunner` happens in
//! TUI-107's `AgentHandle` adapter, not here.
//!
//! ## Spec Deviation (TUI-106-specific)
//!
//! Spec references `TuiEvent::UserInput(prompt)`, `TuiEvent::SlashCancel`,
//! `TuiEvent::SlashAgent(id)` — none of these variants existed in the
//! `TuiEvent` enum before TUI-106. Resolution: three new variants were
//! added additively to [`crate::app::TuiEvent`] (no reordering of
//! existing variants), and corresponding no-op arms were added to the
//! existing `run_tui` match so its exhaustive pattern still compiles.
//! `run_tui` is a no-op on these variants because the new
//! `run_event_loop` is their handler — the old path will be retired by
//! TUI-107.
//!
//! ## Non-blocking contract
//!
//! - No branch of `tokio::select!` calls `.await` on anything in
//!   [`AgentDispatcher`]. `poll_completion` is SYNC by design (see
//!   TUI-103) and is called directly without wrapping in `async {}`.
//! - Both select branches use cancel-safe futures only:
//!   `UnboundedReceiver::recv()` and `tokio::time::Interval::tick()`.
//! - After every `TuiEvent` is handled, `poll_completion` is called
//!   immediately so a turn that finished during the event pump does
//!   NOT wait for the next 16ms tick to drain.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use archon_core::agent::{AgentEvent, TimestampedEvent};
use crossterm::event::{self, Event, KeyCode, KeyModifiers, MouseEventKind};
use ratatui::Terminal;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::app::{
    App, AppConfig, McpManager, McpManagerView, SessionPicker, TuiEvent, should_process_key_event,
};
use crate::task_dispatch::{AgentDispatcher, AgentRouter, CancelOutcome, TurnRunner};
use crate::vim::{VimAction, VimState};

/// Configuration passed to [`run_event_loop`]. Field order and types
/// are pinned by the TUI-106 spec (with TUI-100 deviation for `runner`).
pub struct EventLoopConfig {
    pub tui_event_rx: UnboundedReceiver<TuiEvent>,
    pub agent_event_tx: UnboundedSender<TimestampedEvent>,
    pub runner: Arc<dyn TurnRunner>,
    pub router: Arc<dyn AgentRouter>,
}

/// Main TUI event loop: consume `TuiEvent`s, drive [`AgentDispatcher`],
/// poll completion on a 16ms tick. Returns `Ok(())` when the channel
/// closes or a [`TuiEvent::Done`] is received.
// TUI-330: cognitive complexity (36/25). This is the dispatcher-side event
// loop — a single `select!` over the event channel and a poll interval with
// a match on TuiEvent variants (UserInput, SlashCancel, SlashAgent, Resize,
// Done). Splitting arms into helpers would fragment the match that is the
// architectural focal point of this function and require threading
// dispatcher / runner / router through every helper. Kept as a single
// function intentionally.
//
// TUI-331: Fix 3 attempted extracting a `handle_tui_event(dispatcher, runner,
// ev) -> LoopAction` helper; measured complexity dropped only 36 → 32, still
// over the 25 threshold (the outer `tokio::select!` + `Some/None` match +
// `poll_completion()` drain account for the residual complexity). Refactor
// reverted; allow retained. Remove this allow when either:
//   (a) The outer loop's `tokio::select!` is replaced with a single-source
//       stream abstraction that folds the poll-interval branch into the
//       event channel (removing one level of nesting), OR
//   (b) TUI-107's `AgentHandle` adapter is introduced, at which point the
//       dispatcher / runner / router become fields on a single actor struct
//       and the helper extraction in Fix 3 will land <25.
#[allow(clippy::cognitive_complexity)]
pub async fn run_event_loop(cfg: EventLoopConfig) -> Result<()> {
    let EventLoopConfig {
        mut tui_event_rx,
        agent_event_tx,
        runner,
        router,
    } = cfg;

    let mut dispatcher = AgentDispatcher::new(router, agent_event_tx);
    let mut poll_interval = tokio::time::interval(Duration::from_millis(16));

    loop {
        tokio::select! {
            maybe_ev = tui_event_rx.recv() => {
                match maybe_ev {
                    Some(TuiEvent::UserInput(prompt)) => {
                        let _ = dispatcher.spawn_turn(prompt, runner.clone());
                    }
                    Some(TuiEvent::SlashCancel) => {
                        match dispatcher.cancel_current() {
                            CancelOutcome::NoInflight => {
                                tracing::info!("slash-cancel: no in-flight turn");
                            }
                            CancelOutcome::Aborted { elapsed_ms } => {
                                tracing::info!(elapsed_ms, "slash-cancel: aborted");
                            }
                        }
                    }
                    Some(TuiEvent::SlashAgent(id)) => {
                        match dispatcher.switch_agent(&id) {
                            Ok(()) => tracing::info!(agent = %id, "slash-agent switched"),
                            Err(e) => tracing::warn!(error = %e, agent = %id, "slash-agent failed"),
                        }
                    }
                    Some(TuiEvent::Resize { cols, rows }) => {
                        let _ = crate::layout::handle_resize(cols, rows);
                    }
                    Some(TuiEvent::Done) => break,
                    Some(_) => {
                        // Other TuiEvent variants (agent→TUI output events) are
                        // consumed by the old run_tui path's render loop, not by
                        // this dispatcher-side loop. No-op here.
                    }
                    None => {
                        // Channel closed. Caller dropped the sender.
                        break;
                    }
                }
                // Drain any newly-completed turn in the same frame —
                // do NOT wait for the next 16ms tick.
                let _ = dispatcher.poll_completion();
            }
            _ = poll_interval.tick() => {
                let _ = dispatcher.poll_completion();
            }
        }
    }

    Ok(())
}

/// Backend-generic event loop body (TUI-310 extraction from `app.rs`).
///
/// Shared by [`crate::app::run`] (production crossterm path) and
/// [`crate::app::run_with_backend`] (test injection path).
///
/// **No terminal lifecycle here**: this helper assumes raw mode / alternate
/// screen / mouse capture have already been arranged (or are not needed, for
/// `TestBackend`). Both callers handle their own setup and teardown.
// TUI-330: cognitive complexity (64/25). This is the full inner event loop
// extracted from app.rs under TUI-310 — a match over ~30 TuiEvent variants
// that all mutate shared App state. Extracting arms into per-variant helpers
// would require threading `&mut App` plus several ancillary senders through
// every helper and would fragment the single match arm that is the
// architectural focal point of the loop. Coverage is tracked via the
// TUI-328 80% coverage ratchet.
//
// TUI-331: Remove this allow when either:
//   (a) An `App::process_tui_event(&mut self, event: TuiEvent)` method is
//       introduced that moves the match arms onto `impl App`, so each arm
//       borrows `&mut self` through a single receiver (TUI-311 tracks the
//       input.rs extraction that is step 1 of this path), OR
//   (b) The `App` struct is decomposed into sub-state groups (App::Input,
//       App::Thinking, App::Output, App::Overlays) so variant-specific
//       handlers can accept a narrower `&mut` receiver rather than the
//       current &mut App over 40+ fields.
#[allow(clippy::cognitive_complexity)]
pub(crate) async fn run_inner<B>(
    config: AppConfig,
    terminal: &mut Terminal<B>,
) -> Result<(), io::Error>
where
    B: ratatui::backend::Backend,
{
    let AppConfig {
        mut event_rx,
        input_tx,
        splash,
        btw_tx,
        permission_tx,
    } = config;

    let mut app = App::new();
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

    loop {
        // Draw UI
        terminal.draw(|frame| { crate::render::draw(frame, &mut app) })?;

        // Handle events: use shorter poll when animation is active
        let timeout = if app.input.ultrathink.active || app.thinking.active {
            std::time::Duration::from_millis(80) // 12.5fps — smooth for bounce cycle
        } else {
            std::time::Duration::from_millis(250) // 4fps — poll returns immediately on events
        };

        // Check for agent events (non-blocking)
        while let Ok(tui_event) = event_rx.try_recv() {
            match tui_event {
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

        if app.should_quit {
            break;
        }

        // Check for keyboard input; tick animations on timeout.
        //
        // `event::poll` returns an error in non-tty environments (e.g.
        // integration tests driving the TUI through
        // `run_with_backend` + `TestBackend`): crossterm can't open an
        // input reader without a real stdin. Treat any poll error as
        // "no key available" and fall through to the animation-tick
        // branch — we still honour the timeout by sleeping for it,
        // so scripted event senders get a chance to deliver the next
        // frame worth of events.
        let poll_result = event::poll(timeout);
        let has_event = match poll_result {
            Ok(v) => v,
            Err(_) => {
                tokio::time::sleep(timeout).await;
                false
            }
        };
        if has_event {
            match event::read()? {
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        app.output.scroll_up(3);
                    }
                    MouseEventKind::ScrollDown => {
                        app.output.scroll_down(3);
                    }
                    _ => {}
                },
                Event::Key(key) => {
                    // Windows emits both Press and Release for each keystroke;
                    // process only Press and Repeat to avoid double input.
                    if !should_process_key_event(&key) {
                        continue;
                    }
                    // Handle session picker — Up/Down/Enter/Esc
                    if app.session_picker.is_some() {
                        match key.code {
                            KeyCode::Up => {
                                if let Some(ref mut picker) = app.session_picker {
                                    if picker.selected > 0 {
                                        picker.selected -= 1;
                                    } else {
                                        picker.selected = picker.sessions.len().saturating_sub(1);
                                    }
                                }
                                continue;
                            }
                            KeyCode::Down => {
                                if let Some(ref mut picker) = app.session_picker {
                                    if picker.selected + 1 < picker.sessions.len() {
                                        picker.selected += 1;
                                    } else {
                                        picker.selected = 0;
                                    }
                                }
                                continue;
                            }
                            KeyCode::Enter => {
                                if let Some(picker) = app.session_picker.take()
                                    && let Some(s) = picker.sessions.get(picker.selected)
                                {
                                    let _ =
                                        input_tx.send(format!("__resume_session__ {}", s.id)).await;
                                }
                                continue;
                            }
                            KeyCode::Esc => {
                                app.session_picker = None;
                                continue;
                            }
                            _ => continue, // swallow other keys
                        }
                    }
                    // Handle MCP manager overlay — Up/Down/Enter/Esc
                    if app.mcp_manager.is_some() {
                        match key.code {
                            KeyCode::Up => {
                                if let Some(ref mut mgr) = app.mcp_manager {
                                    match &mut mgr.view {
                                        McpManagerView::ServerList { selected } => {
                                            if *selected > 0 {
                                                *selected -= 1;
                                            } else {
                                                *selected = mgr.servers.len().saturating_sub(1);
                                            }
                                        }
                                        McpManagerView::ServerMenu {
                                            action_idx,
                                            server_idx,
                                        } => {
                                            let count =
                                                mcp_action_count(mgr.servers.get(*server_idx));
                                            if *action_idx > 0 {
                                                *action_idx -= 1;
                                            } else {
                                                *action_idx = count.saturating_sub(1);
                                            }
                                        }
                                        McpManagerView::ToolList { scroll, .. } => {
                                            *scroll = scroll.saturating_sub(1);
                                        }
                                    }
                                }
                                continue;
                            }
                            KeyCode::Down => {
                                if let Some(ref mut mgr) = app.mcp_manager {
                                    match &mut mgr.view {
                                        McpManagerView::ServerList { selected } => {
                                            if *selected + 1 < mgr.servers.len() {
                                                *selected += 1;
                                            } else {
                                                *selected = 0;
                                            }
                                        }
                                        McpManagerView::ServerMenu {
                                            action_idx,
                                            server_idx,
                                        } => {
                                            let action_count =
                                                mcp_action_count(mgr.servers.get(*server_idx));
                                            if *action_idx + 1 < action_count {
                                                *action_idx += 1;
                                            } else {
                                                *action_idx = 0;
                                            }
                                        }
                                        McpManagerView::ToolList { scroll, tools, .. } => {
                                            if *scroll + 1 < tools.len() {
                                                *scroll += 1;
                                            }
                                        }
                                    }
                                }
                                continue;
                            }
                            KeyCode::Enter => {
                                if let Some(ref mut mgr) = app.mcp_manager {
                                    match mgr.view.clone() {
                                        McpManagerView::ServerList { selected } => {
                                            if !mgr.servers.is_empty() {
                                                mgr.view = McpManagerView::ServerMenu {
                                                    server_idx: selected,
                                                    action_idx: 0,
                                                };
                                            }
                                        }
                                        McpManagerView::ServerMenu {
                                            server_idx,
                                            action_idx,
                                        } => {
                                            if let Some(server) = mgr.servers.get(server_idx) {
                                                let actions = mcp_actions_for(server);
                                                if let Some(action) = actions.get(action_idx) {
                                                    match *action {
                                                        "back" => {
                                                            mgr.view = McpManagerView::ServerList {
                                                                selected: server_idx,
                                                            };
                                                        }
                                                        "tools" => {
                                                            mgr.view = McpManagerView::ToolList {
                                                                server_name: server.name.clone(),
                                                                tools: server.tools.clone(),
                                                                scroll: 0,
                                                            };
                                                        }
                                                        _ => {
                                                            let cmd = format!(
                                                                "__mcp_action__ {} {}",
                                                                server.name, action
                                                            );
                                                            let _ = input_tx.send(cmd).await;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        McpManagerView::ToolList { .. } => {
                                            // Enter/Esc handled below — nothing to do on Enter
                                        }
                                    }
                                }
                                continue;
                            }
                            KeyCode::Esc => {
                                if let Some(ref mut mgr) = app.mcp_manager {
                                    match &mgr.view {
                                        McpManagerView::ToolList { server_name, .. } => {
                                            // Find the server index to return to its menu
                                            let idx = mgr
                                                .servers
                                                .iter()
                                                .position(|s| s.name == *server_name)
                                                .unwrap_or(0);
                                            mgr.view = McpManagerView::ServerMenu {
                                                server_idx: idx,
                                                action_idx: 0,
                                            };
                                        }
                                        McpManagerView::ServerMenu { server_idx, .. } => {
                                            let idx = *server_idx;
                                            mgr.view = McpManagerView::ServerList { selected: idx };
                                        }
                                        McpManagerView::ServerList { .. } => {
                                            app.mcp_manager = None;
                                        }
                                    }
                                }
                                continue;
                            }
                            _ => continue, // swallow other keys while overlay is up
                        }
                    }
                    // Handle permission prompt — y/n/Enter/Esc
                    if app.permission_prompt.is_some() {
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                                let tool = app.permission_prompt.take().unwrap_or_default();
                                if let Some(ref tx) = permission_tx {
                                    let _ = tx.send(true).await;
                                }
                                app.output.append_line(&format!("[{tool}: approved]"));
                                continue;
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                let tool = app.permission_prompt.take().unwrap_or_default();
                                if let Some(ref tx) = permission_tx {
                                    let _ = tx.send(false).await;
                                }
                                app.output.append_line(&format!("[{tool}: denied]"));
                                continue;
                            }
                            _ => continue, // swallow other keys during permission prompt
                        }
                    }
                    // Dismiss /btw overlay on any of Esc/Enter/Space
                    if app.btw_overlay.is_some() {
                        match key.code {
                            KeyCode::Esc | KeyCode::Enter | KeyCode::Char(' ') => {
                                app.btw_overlay = None;
                                continue;
                            }
                            _ => continue, // swallow all other keys while overlay is up
                        }
                    }
                    // Vim mode key routing — Ctrl+D / Ctrl+C fall through to normal handling
                    let is_ctrl_quit = key.modifiers == KeyModifiers::CONTROL
                        && matches!(key.code, KeyCode::Char('d') | KeyCode::Char('c'));
                    if !is_ctrl_quit && let Some(ref mut vim) = app.vim_state {
                        let action = vim.handle_key(key);
                        match action {
                            VimAction::Submit => {
                                let text = vim.text();
                                *vim = VimState::new();
                                if !text.trim().is_empty() {
                                    if app.is_generating {
                                        app.pending_input.push(text);
                                        app.output
                                            .append_line("[queued — will send after current turn]");
                                    } else {
                                        let _ = input_tx.send(text).await;
                                    }
                                }
                            }
                            VimAction::Quit => {
                                app.vim_state = None;
                            }
                            _ => {}
                        }
                        continue;
                    }
                    match crate::input::handle_key(&mut app, key, &keymap) {
                        crate::input::KeyResult::Nothing => {}
                        crate::input::KeyResult::Quit => {
                            app.should_quit = true;
                        }
                        crate::input::KeyResult::SendInput(text) => {
                            let _ = input_tx.send(text).await;
                        }
                        crate::input::KeyResult::SendCancel => {
                            let _ = input_tx.try_send("__cancel__".to_string());
                        }
                        crate::input::KeyResult::SendBtw(q) => {
                            if let Some(ref btw) = btw_tx {
                                let _ = btw.send(q).await;
                            }
                        }
                    }
                }
                Event::Resize(cols, rows) => {
                    crate::layout::handle_resize(cols, rows);
                }
                _ => {} // FocusGained/FocusLost/Paste
            }
        } else {
            // No key event — tick animations
            app.input.ultrathink.tick();
            app.thinking.tick_thinking();
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
