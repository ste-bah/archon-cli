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

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use archon_core::agent::{AgentEvent, TimestampedEvent};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::app::TuiEvent;
use crate::task_dispatch::{AgentDispatcher, AgentRouter, CancelOutcome, TurnRunner};

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
