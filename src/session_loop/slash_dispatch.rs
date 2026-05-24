use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use archon_core::agent::Agent;
use archon_core::skills::{SkillContext, SkillOutput};
use archon_llm::effort::EffortState;
use archon_llm::fast_mode::FastModeState;
use archon_tui::app::TuiEvent;

use super::personality_save::save_personality_snapshot_if_enabled;
use super::post_turn::PostTurnAction;
use super::session_export::drain_pending_export;
use super::slash_handlers::{handle_clear_command, handle_refresh_identity_command};
use crate::command::slash::handle_slash_command;
use crate::slash_context::SlashCommandContext;

pub(super) struct SlashDispatchContext<'a> {
    pub(super) agent: &'a Arc<tokio::sync::Mutex<Agent>>,
    pub(super) api_url: &'a Option<String>,
    pub(super) input_tui_tx: &'a archon_tui::event_channel::TuiEventSender,
    pub(super) session_store: &'a Arc<archon_session::storage::SessionStore>,
    pub(super) session_id: &'a str,
    pub(super) persist_personality: bool,
    pub(super) personality_history_limit: u32,
    pub(super) session_start_confidence: f32,
    pub(super) session_start_instant: Instant,
    pub(super) fast_mode: &'a mut FastModeState,
    pub(super) effort_state: &'a mut EffortState,
    pub(super) cmd_ctx: &'a mut SlashCommandContext,
    pub(super) dispatcher: &'a Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>,
    pub(super) adapter: &'a Arc<crate::agent_handle::AgentHandle>,
    pub(super) post_turn_queue: &'a mut VecDeque<PostTurnAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SlashDispatchResult {
    Handled,
    Unhandled,
}

impl SlashDispatchResult {
    pub(super) fn is_handled(self) -> bool {
        matches!(self, Self::Handled)
    }
}

pub(super) async fn dispatch_slash_or_skill(
    input: &str,
    ctx: SlashDispatchContext<'_>,
) -> SlashDispatchResult {
    let trimmed = input.trim();
    if matches!(trimmed, "/exit" | "/quit" | "/q") {
        handle_exit(&ctx).await;
        return SlashDispatchResult::Handled;
    }
    if trimmed == "/compact" || trimmed.starts_with("/compact ") {
        handle_compact(trimmed, &ctx).await;
        return SlashDispatchResult::Handled;
    }
    if trimmed == "/clear" {
        handle_clear_command(
            ctx.agent,
            ctx.cmd_ctx,
            ctx.input_tui_tx,
            ctx.session_store,
            ctx.session_id,
            ctx.persist_personality,
            ctx.personality_history_limit,
            ctx.session_start_confidence,
            ctx.session_start_instant,
        )
        .await;
        return SlashDispatchResult::Handled;
    }
    if trimmed == "/refresh-identity" {
        handle_refresh_identity_command(ctx.agent, ctx.api_url, ctx.input_tui_tx).await;
        return SlashDispatchResult::Handled;
    }

    let handled = handle_slash_command(
        trimmed,
        &mut *ctx.fast_mode,
        &mut *ctx.effort_state,
        ctx.input_tui_tx,
        &mut *ctx.cmd_ctx,
    )
    .await;
    if handled {
        drain_pending_export(ctx.agent, ctx.cmd_ctx, ctx.input_tui_tx).await;
        return SlashDispatchResult::Handled;
    }

    if handle_skill_fallback(trimmed, ctx).await {
        SlashDispatchResult::Handled
    } else {
        SlashDispatchResult::Unhandled
    }
}

async fn handle_exit(ctx: &SlashDispatchContext<'_>) {
    let iv_arc = ctx.agent.lock().await.inner_voice().cloned();
    save_personality_snapshot_if_enabled(
        iv_arc,
        ctx.cmd_ctx.memory.as_ref(),
        &ctx.cmd_ctx.session_id,
        ctx.persist_personality,
        ctx.personality_history_limit,
        ctx.session_start_confidence,
        ctx.session_start_instant,
    )
    .await;

    {
        let guard = ctx.agent.lock().await;
        guard.fire_hook_detached(
            archon_core::hooks::HookType::SessionEnd,
            serde_json::json!({"hook_type": "session_end", "reason": "exit"}),
        )
    }
    .await;
    ctx.agent.lock().await.clear_watch_paths();
    let _ = ctx
        .input_tui_tx
        .send(TuiEvent::TextDelta("\nGoodbye.\n".into()));
    let _ = ctx.input_tui_tx.send(TuiEvent::Done);
}

async fn handle_compact(trimmed: &str, ctx: &SlashDispatchContext<'_>) {
    let subcommand = trimmed.strip_prefix("/compact").unwrap().trim();
    let subcommand = if subcommand.is_empty() {
        None
    } else {
        Some(subcommand)
    };
    let (outcome, compacted_messages) = {
        let mut guard = ctx.agent.lock().await;
        let fut: std::pin::Pin<
            Box<
                dyn std::future::Future<Output = archon_core::agent::ManualCompactOutcome>
                    + Send
                    + '_,
            >,
        > = Box::pin(guard.compact(subcommand));
        let outcome = fut.await;
        let messages = if matches!(
            outcome,
            archon_core::agent::ManualCompactOutcome::Compacted { .. }
        ) {
            guard
                .conversation_state()
                .messages
                .iter()
                .filter_map(|msg| serde_json::to_string(msg).ok())
                .collect()
        } else {
            Vec::new()
        };
        (outcome, messages)
    };
    if !compacted_messages.is_empty()
        && let Err(e) = ctx
            .session_store
            .replace_messages(ctx.session_id, &compacted_messages)
    {
        tracing::warn!("replace_messages after /compact failed: {e}");
    }
    let msg = outcome.into_status();
    let _ = ctx
        .input_tui_tx
        .send(TuiEvent::TextDelta(format!("\n{msg}\n")));
    let _ = ctx.input_tui_tx.send(TuiEvent::SlashCommandComplete);
}

async fn handle_skill_fallback(input: &str, ctx: SlashDispatchContext<'_>) -> bool {
    let (cmd_name, cmd_args) = match archon_core::skills::parser::parse_slash_command(input) {
        Some((name, args)) => (name, args),
        None => (String::new(), Vec::new()),
    };
    let skill_output: Option<SkillOutput> = {
        let skill = ctx.cmd_ctx.skill_registry.resolve(&cmd_name);
        skill.map(|s| {
            let skill_ctx = SkillContext {
                session_id: ctx.cmd_ctx.session_id.clone(),
                working_dir: ctx.cmd_ctx.working_dir.clone(),
                model: ctx.cmd_ctx.default_model.clone(),
                agent_registry: Some(Arc::clone(&ctx.cmd_ctx.agent_registry)),
                session_store: Some(Arc::clone(&ctx.cmd_ctx.session_store)),
            };
            s.execute(&cmd_args, &skill_ctx)
        })
    };
    let Some(output) = skill_output else {
        return false;
    };
    emit_skill_output(cmd_name, output, ctx).await;
    true
}

async fn emit_skill_output(cmd_name: String, output: SkillOutput, ctx: SlashDispatchContext<'_>) {
    match output {
        SkillOutput::Prompt(prompt) => {
            {
                let mut resp = ctx.cmd_ctx.last_assistant_response.lock().await;
                resp.clear();
            }
            let _ = ctx.input_tui_tx.send(TuiEvent::GenerationStarted);
            match ctx.dispatcher.lock().unwrap().spawn_turn(
                prompt,
                ctx.adapter.clone() as Arc<dyn archon_tui::TurnRunner>,
            ) {
                archon_tui::DispatchResult::Running { .. } => {
                    tracing::debug!("spawned skill agent turn");
                }
                archon_tui::DispatchResult::Queued => {
                    tracing::debug!("agent busy; queued skill prompt");
                }
                archon_tui::DispatchResult::Rejected(err) => {
                    tracing::error!("skill dispatch rejected: {err}");
                }
            }
            ctx.post_turn_queue
                .push_back(PostTurnAction::SkillComplete {
                    reload_registry_for: Some(cmd_name),
                });
        }
        SkillOutput::Text(text) | SkillOutput::Markdown(text) => {
            let _ = ctx
                .input_tui_tx
                .send(TuiEvent::TextDelta(format!("\n{text}\n")));
            let _ = ctx.input_tui_tx.send(TuiEvent::SlashCommandComplete);
        }
        SkillOutput::Error(error) => {
            let _ = ctx
                .input_tui_tx
                .send(TuiEvent::TextDelta(format!("\nError: {error}\n")));
            let _ = ctx.input_tui_tx.send(TuiEvent::SlashCommandComplete);
        }
    }
}
