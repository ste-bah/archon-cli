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
//!
//! ## REM-2g: file-split layout
//!
//! Relocated from `src/event_loop.rs` → `src/event_loop/` per REM-2g
//! (docs/rem-2-split-plan.md section 3.3). Public API unchanged:
//! `EventLoopConfig`, `run_event_loop` stay in `mod.rs`; `lib.rs` keeps
//! `pub use event_loop::{EventLoopConfig, run_event_loop}`.
//!
//! Submodules (private to `event_loop`):
//! - `tui_events` — `handle_tui_event` (30-arm `TuiEvent` drain match)
//! - `input` — `handle_key_event` (crossterm keyboard/mouse/resize dispatch)
//!
//! `run_inner` now delegates its two per-iteration branches to these
//! helpers. `mcp_actions_for` / `mcp_action_count` live here so both
//! the outer loop and `input::handle_key_event` can reference them via
//! `super::`.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use archon_core::agent::{AgentEvent, TimestampedEvent};
use crossterm::event::{self, Event};
use ratatui::Terminal;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::app::{App, AppConfig, TuiEvent};
use crate::task_dispatch::{AgentDispatcher, AgentRouter, CancelOutcome, TurnRunner};

mod input;
mod tui_events;

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
    let AppConfig {
        mut event_rx,
        input_tx,
        splash,
        btw_tx,
        permission_tx,
        command_catalog,
    } = config;

    crate::commands::set_catalog(command_catalog);

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
        terminal.draw(|frame| crate::render::draw(frame, &mut app))?;

        // Handle events: use shorter poll when animation is active
        let timeout = if app.input.ultrathink.active || app.thinking.active {
            std::time::Duration::from_millis(80) // 12.5fps — smooth for bounce cycle
        } else {
            std::time::Duration::from_millis(250) // 4fps — poll returns immediately on events
        };

        // Check for agent events (non-blocking)
        while let Ok(tui_event) = event_rx.try_recv() {
            tui_events::handle_tui_event(&mut app, tui_event, &input_tx).await;
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
            let ev = event::read()?;
            input::handle_key_event(
                &mut app,
                ev,
                &input_tx,
                btw_tx.as_ref(),
                permission_tx.as_ref(),
                &keymap,
            )
            .await;
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
