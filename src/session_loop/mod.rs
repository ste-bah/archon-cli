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

use std::borrow::Cow;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
pub(crate) mod session_history;
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
    sandbox_audit_drain: crate::runtime::sandbox_audit_writer::SandboxAuditDrainHandle,
    handle_process_signals: bool,
    shutdown: tokio_util::sync::CancellationToken,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>> {
    Box::pin(async move {
        let agent = Arc::new(tokio::sync::Mutex::new(agent));

        fire_session_startup_hooks(&agent).await;

        if let Some(ref def) = agent_def {
            input_tui_tx
                .send_async(TuiEvent::SetAgentInfo {
                    name: def.agent_type.clone(),
                    color: def.color.clone(),
                })
                .await?;
        }

        let mut initial_prompt_pending: Option<String> =
            agent_def.as_ref().and_then(|d| d.initial_prompt.clone());

        let adapter = Arc::new(crate::agent_handle::AgentHandle::new(
            Arc::clone(&agent),
            session_id_for_input.clone(),
            auto_capture,
            auto_trainer.clone(),
        ));
        *cancel_handle_slot.lock().unwrap() = Some(Arc::clone(&adapter));
        let activity_cwd = cmd_ctx.working_dir.clone();
        crate::command::cognitive_daemon::record_archon_activity(
            &config,
            &activity_cwd,
            "session_start",
        );
        let mut last_busy_activity = Instant::now();

        let mut post_turn_queue: std::collections::VecDeque<PostTurnAction> =
            std::collections::VecDeque::new();
        let mut poll_tick = tokio::time::interval(std::time::Duration::from_millis(16));
        poll_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        #[cfg(unix)]
        let (mut sigterm_stream, signal_registration_error) = if handle_process_signals {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => (Some(signal), None),
                Err(error) => (
                    None,
                    Some(anyhow::anyhow!("install SIGTERM handler failed: {error}")),
                ),
            }
        } else {
            (None, None)
        };
        let shutdown_in_progress = std::sync::atomic::AtomicBool::new(false);
        #[cfg(unix)]
        if let Some(error) = signal_registration_error {
            let shutdown_result =
                finish_session(&agent_def, &agent, &agent_dispatcher, sandbox_audit_drain).await;
            return finish_loop_result(shutdown_result, Some(error));
        }

        let mut loop_error = None;
        loop {
            if last_busy_activity.elapsed() >= Duration::from_secs(30)
                && dispatcher_has_work(&agent_dispatcher)
            {
                crate::command::cognitive_daemon::record_archon_activity(
                    &config,
                    &activity_cwd,
                    "agent_busy",
                );
                last_busy_activity = Instant::now();
            }

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
                handle_process_signals,
                shutdown: &shutdown,
            })
            .await
            {
                LoopInput::Input(input) => input,
                LoopInput::Continue => continue,
                LoopInput::Stop => break,
                LoopInput::Error(error) => {
                    loop_error = Some(error);
                    break;
                }
            };
            crate::command::cognitive_daemon::record_archon_activity(
                &config,
                &activity_cwd,
                "user_input",
            );

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

            if let Some(slash_input) = slash_input(&input) {
                let slash_input = slash_input.as_ref();
                if slash_commands_disabled {
                    tracing::warn!(
                        command = slash_command_name(slash_input),
                        input_len = slash_input.len(),
                        "slash command rejected because slash commands are disabled"
                    );
                    if let Err(error) = input_tui_tx
                        .send_async(TuiEvent::Error(format!(
                            "Slash command `{}` was not run because slash commands are disabled.",
                            slash_command_name(slash_input)
                        )))
                        .await
                    {
                        loop_error = Some(error.into());
                        break;
                    }
                    if let Err(error) = input_tui_tx
                        .send_async(TuiEvent::SlashCommandComplete)
                        .await
                    {
                        loop_error = Some(error.into());
                        break;
                    }
                    continue;
                } else {
                    tracing::info!(
                        command = slash_command_name(slash_input),
                        input_len = slash_input.len(),
                        "slash command dispatch starting"
                    );
                    let dispatch_result = dispatch_slash_or_skill(
                        slash_input,
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
                    .await;
                    if dispatch_result.is_handled() {
                        tracing::info!(
                            command = slash_command_name(slash_input),
                            "slash command dispatch handled"
                        );
                        if dispatch_result.should_exit() {
                            break;
                        }
                        continue;
                    }
                    tracing::warn!(
                        command = slash_command_name(slash_input),
                        "slash command dispatch unhandled"
                    );
                    if let Err(error) = input_tui_tx
                        .send_async(TuiEvent::TextDelta(format!(
                            "\nUnknown slash command `{}`. Type /help for available commands.\n",
                            slash_command_name(slash_input)
                        )))
                        .await
                    {
                        loop_error = Some(error.into());
                        break;
                    }
                    if let Err(error) = input_tui_tx
                        .send_async(TuiEvent::SlashCommandComplete)
                        .await
                    {
                        loop_error = Some(error.into());
                        break;
                    }
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

        let shutdown_result =
            finish_session(&agent_def, &agent, &agent_dispatcher, sandbox_audit_drain).await;
        finish_loop_result(shutdown_result, loop_error)
    })
}

fn finish_loop_result(
    shutdown_result: anyhow::Result<()>,
    loop_error: Option<anyhow::Error>,
) -> anyhow::Result<()> {
    match (loop_error, shutdown_result) {
        (None, Ok(())) => Ok(()),
        (Some(loop_error), Ok(())) => Err(loop_error),
        (None, Err(shutdown_error)) => Err(shutdown_error),
        (Some(loop_error), Err(shutdown_error)) => Err(anyhow::anyhow!(
            "session loop failed: {loop_error:#}; session shutdown failed: {shutdown_error:#}"
        )),
    }
}

fn dispatcher_has_work(dispatcher: &Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>) -> bool {
    let dispatcher = dispatcher.lock().unwrap();
    dispatcher.is_busy() || dispatcher.queue_len() > 0
}

fn slash_input(input: &str) -> Option<Cow<'_, str>> {
    let trimmed = input.trim_start_matches(|ch: char| ch.is_whitespace() || ch == '\u{feff}');
    let prompt_stripped = strip_prompt_marker(trimmed);
    if let Some(workflow) =
        workflow_cli_input(prompt_stripped).or_else(|| workflow_cli_input(trimmed))
    {
        return Some(workflow);
    }
    if prompt_stripped.starts_with('/') {
        return Some(Cow::Borrowed(prompt_stripped));
    }
    if trimmed.starts_with('/') {
        return Some(Cow::Borrowed(trimmed));
    }
    None
}

fn strip_prompt_marker(input: &str) -> &str {
    [">", "$", "%"]
        .iter()
        .find_map(|marker| {
            input.strip_prefix(marker).and_then(|rest| {
                rest.chars()
                    .next()
                    .is_some_and(char::is_whitespace)
                    .then_some(rest)
            })
        })
        .map(str::trim_start)
        .unwrap_or(input)
}

fn workflow_cli_input(input: &str) -> Option<Cow<'_, str>> {
    let executable = input.split_whitespace().next()?;
    let after_executable = input.strip_prefix(executable)?.trim_start();
    let command = after_executable.split_whitespace().next()?;
    if command != "workflow" || !is_archon_executable(executable) {
        return None;
    }
    let rest = after_executable
        .strip_prefix(command)
        .unwrap_or("")
        .trim_start();
    if rest.is_empty() {
        Some(Cow::Borrowed("/workflow"))
    } else {
        Some(Cow::Owned(format!("/workflow {rest}")))
    }
}

fn is_archon_executable(executable: &str) -> bool {
    matches!(executable, "archon" | "./archon") || executable.ends_with("/archon")
}

fn slash_command_name(input: &str) -> &str {
    input.split_whitespace().next().unwrap_or(input)
}

#[cfg(test)]
mod tests {
    use super::{finish_loop_result, slash_command_name, slash_input};

    #[test]
    fn signal_registration_and_shutdown_failures_remain_visible() {
        let result = finish_loop_result(
            Err(anyhow::anyhow!("audit shutdown failed")),
            Some(anyhow::anyhow!("SIGTERM registration failed")),
        )
        .expect_err("signal and shutdown failures must remain visible");
        let message = result.to_string();

        assert!(
            message.contains("SIGTERM registration failed"),
            "{result:#}"
        );
        assert!(message.contains("audit shutdown failed"), "{result:#}");
    }

    #[test]
    fn finish_loop_reports_loop_and_shutdown_failures_together() {
        let result = finish_loop_result(
            Err(anyhow::anyhow!("audit shutdown failed")),
            Some(anyhow::anyhow!("loop failed")),
        )
        .expect_err("both session-loop failures must remain visible");
        let message = result.to_string();

        assert!(message.contains("loop failed"), "{result:#}");
        assert!(message.contains("audit shutdown failed"), "{result:#}");
    }

    #[test]
    fn slash_input_allows_leading_whitespace() {
        assert_eq!(
            slash_input("  /cognitive daemon start"),
            Some(std::borrow::Cow::Borrowed("/cognitive daemon start"))
        );
    }

    #[test]
    fn slash_input_rejects_plain_prompt() {
        assert_eq!(slash_input("hello /cognitive"), None);
    }

    #[test]
    fn slash_command_name_returns_first_token() {
        assert_eq!(slash_command_name("/cognitive daemon start"), "/cognitive");
    }

    #[test]
    fn slash_input_accepts_copied_tui_prompt_marker() {
        assert_eq!(
            slash_input("> /workflow run --live build it"),
            Some(std::borrow::Cow::Borrowed("/workflow run --live build it"))
        );
    }

    #[test]
    fn slash_input_normalizes_tui_cli_workflow_command() {
        assert_eq!(
            slash_input("./archon workflow resume --live wf-123"),
            Some(std::borrow::Cow::Owned(
                "/workflow resume --live wf-123".to_string()
            ))
        );
    }

    #[test]
    fn slash_input_normalizes_absolute_cli_workflow_command() {
        assert_eq!(
            slash_input("/tmp/project/archon workflow run --live do work"),
            Some(std::borrow::Cow::Owned(
                "/workflow run --live do work".to_string()
            ))
        );
    }

    #[test]
    fn slash_input_preserves_decomposed_workflow_flag() {
        assert_eq!(
            slash_input("./archon workflow run --live --decomposed do work"),
            Some(std::borrow::Cow::Owned(
                "/workflow run --live --decomposed do work".to_string()
            ))
        );
    }
}
