use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use archon_core::agent::Agent;

use super::post_turn::{PostTurnAction, handle_completed_turn};
use crate::slash_context::SlashCommandContext;

pub(super) enum LoopInput {
    Input(String),
    Continue,
    Stop,
    Error(anyhow::Error),
}

pub(super) struct LoopInputContext<'a> {
    pub(super) poll_tick: &'a mut tokio::time::Interval,
    pub(super) user_input_rx: &'a mut tokio::sync::mpsc::Receiver<String>,
    #[cfg(unix)]
    pub(super) sigterm_stream: &'a mut Option<tokio::signal::unix::Signal>,
    pub(super) shutdown_in_progress: &'a AtomicBool,
    pub(super) agent: &'a Arc<tokio::sync::Mutex<Agent>>,
    pub(super) config: &'a archon_core::config::ArchonConfig,
    pub(super) input_tui_tx: &'a archon_tui::event_channel::TuiEventSender,
    pub(super) session_store: &'a Arc<archon_session::storage::SessionStore>,
    pub(super) session_id: &'a str,
    pub(super) dispatcher: &'a Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>,
    pub(super) adapter: &'a Arc<crate::agent_handle::AgentHandle>,
    pub(super) cmd_ctx: &'a mut SlashCommandContext,
    pub(super) post_turn_queue: &'a mut VecDeque<PostTurnAction>,
    pub(super) handle_process_signals: bool,
    pub(super) shutdown: &'a tokio_util::sync::CancellationToken,
}

pub(super) async fn next_loop_input(ctx: LoopInputContext<'_>) -> LoopInput {
    tokio::select! {
        biased;
        _ = ctx.poll_tick.tick() => {
            poll_completed_turn(ctx).await;
            LoopInput::Continue
        }
        _ = ctx.shutdown.cancelled() => LoopInput::Stop,
        maybe_input = ctx.user_input_rx.recv() => {
            match maybe_input {
                Some(input) => LoopInput::Input(input),
                None => LoopInput::Stop,
            }
        }
        result = async {
            if ctx.handle_process_signals {
                tokio::signal::ctrl_c().await
            } else {
                std::future::pending().await
            }
        } => {
            match result {
                Ok(()) => shutdown_input(ctx.shutdown_in_progress, "SIGINT"),
                Err(error) => LoopInput::Error(anyhow::anyhow!(
                    "install Ctrl-C handler failed: {error}"
                )),
            }
        }
        _ = async {
            #[cfg(unix)]
            {
                if let Some(s) = ctx.sigterm_stream.as_mut() {
                    s.recv().await;
                } else {
                    std::future::pending::<()>().await
                }
            }
            #[cfg(not(unix))]
            {
                std::future::pending::<()>().await
            }
        } => {
            shutdown_input(ctx.shutdown_in_progress, "SIGTERM")
        }
    }
}

async fn poll_completed_turn(ctx: LoopInputContext<'_>) {
    let outcome = ctx.dispatcher.lock().unwrap().poll_completion();
    let Some(outcome) = outcome else {
        return;
    };
    tracing::debug!(
        "dispatcher turn outcome: {}",
        match &outcome {
            archon_tui::TurnOutcome::Completed => "completed",
            archon_tui::TurnOutcome::Cancelled => "cancelled",
            archon_tui::TurnOutcome::FinalizationBlocked(_) => "finalization_blocked",
            archon_tui::TurnOutcome::Failed(_) => "failed",
        }
    );
    handle_completed_turn(
        outcome,
        ctx.post_turn_queue,
        ctx.agent,
        ctx.config,
        ctx.input_tui_tx,
        ctx.session_store,
        ctx.session_id,
        ctx.dispatcher,
        ctx.adapter,
        ctx.cmd_ctx,
    )
    .await;
}

fn shutdown_input(shutdown_in_progress: &AtomicBool, signal_name: &str) -> LoopInput {
    if shutdown_in_progress.swap(true, Ordering::SeqCst) {
        return LoopInput::Continue;
    }
    tracing::info!("{signal_name} received; routing through /exit");
    LoopInput::Input("/exit".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_shutdown_signal_admits_single_exit() {
        let shutdown_in_progress = AtomicBool::new(false);

        assert!(matches!(
            shutdown_input(&shutdown_in_progress, "SIGINT"),
            LoopInput::Input(input) if input == "/exit"
        ));
        assert!(matches!(
            shutdown_input(&shutdown_in_progress, "SIGTERM"),
            LoopInput::Continue
        ));
    }
}
