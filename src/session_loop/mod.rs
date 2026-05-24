//! Session input-loop extracted from `session.rs`.
//!
//! This module hosts `run_session_loop` — the 900-line body that used
//! to live inside a single `tokio::spawn(async move { ... })` block at
//! `src/session.rs:1959`. Extraction into a named `async fn` with
//! explicit owned parameters was required to unblock the
//! `archon-cli-workspace` bin build: three cascading
//! "Send is not general enough" HRTB errors surfaced when rustc tried
//! to infer Send bounds for the anonymous `async move` future. A
//! named function's signature gives each parameter a concrete type
//! for Send analysis, eliminating the HRTB inference failure.
//!
//! ZERO SEMANTIC CHANGE: the body is a verbatim move of the original
//! spawn block. All captured bindings are now owned parameters (or
//! `Arc<T>` — still owned, just shared). Follow-up
//! `TASK-SESSION-LOOP-SPLIT` will break this file into per-event
//! helper modules (hooks, tui_events, slash_commands). See the
//! commit body for the full rationale.

use std::sync::Arc;

use archon_core::agent::Agent;
use archon_llm::effort::EffortState;
use archon_llm::fast_mode::FastModeState;
use archon_pipeline::capture::AutoCapture;
use archon_tui::app::TuiEvent;

use crate::slash_context::SlashCommandContext;

mod control_input;
mod lifecycle_hooks;
mod loop_input;
mod mcp_task;
mod personality_save;
mod post_turn;
mod prompt_turn;
mod session_export;
mod session_history;
mod session_shutdown;
mod slash_dispatch;
mod slash_handlers;

use control_input::{ControlInputContext, handle_control_input};
use lifecycle_hooks::fire_session_startup_hooks;
use loop_input::{LoopInput, LoopInputContext, next_loop_input};
pub(crate) use mcp_task::{McpLifecycleTx, spawn_mcp_lifecycle_task};
use post_turn::PostTurnAction;
use prompt_turn::dispatch_user_prompt;
use session_shutdown::finish_session;
use slash_dispatch::{SlashDispatchContext, dispatch_slash_or_skill};

/// Run the interactive agent input loop to completion.
///
/// TASK-SESSION-LOOP-EXTRACT (A-2): returns an explicit
/// `Pin<Box<dyn Future + Send>>` (not `async fn` → `impl Future`).
/// The A-2 channel flip removed the `&Sender<TuiEvent>` HRTB error,
/// but the async body still holds `&mut SlashCommandContext` /
/// `&str` borrows across many `.await` sites, and rustc's
/// higher-ranked Send inference fails on those patterns
/// (rust-lang/rust#102211). The explicit trait-object return type
/// forces rustc to use the concrete boxed-future type for Send
/// analysis — `tokio::spawn(run_session_loop(..))` then type-checks
/// concretely. Zero semantic change.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_session_loop(
    agent: Agent,
    config: archon_core::config::ArchonConfig,
    agent_def: Option<archon_core::agents::CustomAgentDefinition>,
    api_url: Option<String>,
    input_tui_tx: archon_tui::event_channel::TuiEventSender,
    mut user_input_rx: tokio::sync::mpsc::Receiver<String>,
    session_store_for_input: Arc<archon_session::storage::SessionStore>,
    session_id_for_input: String,
    persist_personality: bool,
    personality_history_limit: u32,
    session_start_instant: std::time::Instant,
    session_start_confidence: f32,
    slash_commands_disabled: bool,
    mut fast_mode: FastModeState,
    mut effort_state: EffortState,
    mut cmd_ctx: SlashCommandContext,
    mcp_lifecycle_tx: McpLifecycleTx,
    auto_capture: Option<Arc<AutoCapture>>,
    auto_trainer: Option<Arc<archon_pipeline::learning::gnn::auto_trainer::AutoTrainer>>,
    agent_dispatcher: Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>,
    cancel_handle_slot: Arc<std::sync::Mutex<Option<Arc<crate::agent_handle::AgentHandle>>>>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        let agent = Arc::new(tokio::sync::Mutex::new(agent));

        fire_session_startup_hooks(&agent).await;

        if let Some(ref def) = agent_def {
            let _ = input_tui_tx.send(TuiEvent::SetAgentInfo {
                name: def.agent_type.clone(),
                color: def.color.clone(),
            });
        }

        let mut initial_prompt_pending: Option<String> =
            agent_def.as_ref().and_then(|d| d.initial_prompt.clone());

        let adapter = Arc::new(crate::agent_handle::AgentHandle::new(
            Arc::clone(&agent),
            auto_capture,
            auto_trainer.clone(),
        ));
        *cancel_handle_slot.lock().unwrap() = Some(Arc::clone(&adapter));

        let mut post_turn_queue: std::collections::VecDeque<PostTurnAction> =
            std::collections::VecDeque::new();
        let mut poll_tick = tokio::time::interval(std::time::Duration::from_millis(16));
        poll_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        #[cfg(unix)]
        let mut sigterm_stream =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::warn!("install SIGTERM handler failed: {e}");
                    None
                }
            };
        let shutdown_in_progress = std::sync::atomic::AtomicBool::new(false);

        loop {
            let input = match next_loop_input(LoopInputContext {
                poll_tick: &mut poll_tick,
                user_input_rx: &mut user_input_rx,
                #[cfg(unix)]
                sigterm_stream: &mut sigterm_stream,
                shutdown_in_progress: &shutdown_in_progress,
                agent: &agent,
                config: &config,
                input_tui_tx: &input_tui_tx,
                session_store: &session_store_for_input,
                session_id: &session_id_for_input,
                dispatcher: &agent_dispatcher,
                adapter: &adapter,
                cmd_ctx: &mut cmd_ctx,
                post_turn_queue: &mut post_turn_queue,
            })
            .await
            {
                LoopInput::Input(input) => input,
                LoopInput::Continue => continue,
                LoopInput::Stop => break,
            };

            if handle_control_input(
                &input,
                ControlInputContext {
                    agent: &agent,
                    input_tui_tx: &input_tui_tx,
                    session_store: &session_store_for_input,
                    session_id: &session_id_for_input,
                    adapter: &adapter,
                    dispatcher: &agent_dispatcher,
                    mcp_lifecycle_tx: &mcp_lifecycle_tx,
                    cmd_ctx: &cmd_ctx,
                },
            )
            .await
            {
                continue;
            }

            if !slash_commands_disabled && input.starts_with('/') {
                if dispatch_slash_or_skill(
                    &input,
                    SlashDispatchContext {
                        agent: &agent,
                        api_url: &api_url,
                        input_tui_tx: &input_tui_tx,
                        session_store: &session_store_for_input,
                        session_id: &session_id_for_input,
                        persist_personality,
                        personality_history_limit,
                        session_start_confidence,
                        session_start_instant,
                        fast_mode: &mut fast_mode,
                        effort_state: &mut effort_state,
                        cmd_ctx: &mut cmd_ctx,
                        dispatcher: &agent_dispatcher,
                        adapter: &adapter,
                        post_turn_queue: &mut post_turn_queue,
                    },
                )
                .await
                .is_handled()
                {
                    continue;
                }
            }

            dispatch_user_prompt(
                input,
                &mut initial_prompt_pending,
                &mut post_turn_queue,
                &agent,
                &config,
                &input_tui_tx,
                &cmd_ctx,
                &session_id_for_input,
                &agent_dispatcher,
                &adapter,
            )
            .await;
        }

        finish_session(&agent_def, &agent, &agent_dispatcher).await;
    })
}
