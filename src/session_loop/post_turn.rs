use std::collections::VecDeque;
use std::sync::Arc;

use archon_core::agent::Agent;
use archon_tui::app::TuiEvent;

use crate::slash_context::SlashCommandContext;

pub(super) enum PostTurnAction {
    PersistSession {
        guardrail: Option<crate::command::world_model::RuntimeGuardrailRecord>,
    },
    SkillComplete {
        reload_registry_for: Option<String>,
    },
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_completed_turn(
    outcome: archon_tui::TurnOutcome,
    queue: &mut VecDeque<PostTurnAction>,
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    config: &archon_core::config::ArchonConfig,
    input_tui_tx: &archon_tui::event_channel::TuiEventSender,
    session_store: &Arc<archon_session::storage::SessionStore>,
    session_id: &str,
    dispatcher: &Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>,
    adapter: &Arc<crate::agent_handle::AgentHandle>,
    cmd_ctx: &mut SlashCommandContext,
) {
    match queue.pop_front() {
        Some(PostTurnAction::PersistSession { guardrail }) => {
            persist_session_messages(agent, session_store, session_id).await;
            if let Some(guardrail) = guardrail {
                maybe_spawn_guardrail_repair(
                    outcome,
                    config,
                    input_tui_tx,
                    dispatcher,
                    adapter,
                    queue,
                    guardrail,
                );
            }
        }
        Some(PostTurnAction::SkillComplete {
            reload_registry_for,
        }) => {
            if reload_registry_for.as_deref() == Some("create-agent")
                && let Ok(mut registry) = cmd_ctx.agent_registry.write()
            {
                registry.reload(&cmd_ctx.working_dir);
                tracing::info!("agent registry reloaded");
            }
            let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete);
        }
        None => {}
    }
}

async fn persist_session_messages(
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    session_store: &Arc<archon_session::storage::SessionStore>,
    session_id: &str,
) {
    let guard = agent.lock().await;
    let messages: Vec<String> = guard
        .conversation_state()
        .messages
        .iter()
        .filter_map(|msg| serde_json::to_string(msg).ok())
        .collect();
    drop(guard);
    if !messages.is_empty()
        && let Err(error) = session_store.replace_messages(session_id, &messages)
    {
        tracing::warn!("replace_messages post-turn failed: {error}");
    }
}

#[allow(clippy::too_many_arguments)]
fn maybe_spawn_guardrail_repair(
    outcome: archon_tui::TurnOutcome,
    config: &archon_core::config::ArchonConfig,
    input_tui_tx: &archon_tui::event_channel::TuiEventSender,
    dispatcher: &Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>,
    adapter: &Arc<crate::agent_handle::AgentHandle>,
    queue: &mut VecDeque<PostTurnAction>,
    guardrail: crate::command::world_model::RuntimeGuardrailRecord,
) {
    let completed = matches!(outcome, archon_tui::TurnOutcome::Completed);
    let guardrail_outcome =
        crate::command::world_model::record_guardrail_turn_outcome(config, &guardrail, completed);
    if completed
        && guardrail_outcome.as_ref().is_some_and(|outcome| {
            matches!(
                outcome.final_status,
                archon_world_model::GuardrailFinalStatus::BlockedMissingVerification
                    | archon_world_model::GuardrailFinalStatus::BlockedFailedVerification
            )
        })
        && let Some(repair_prompt) = crate::command::world_model::forced_repair_prompt(&guardrail)
    {
        let _ = input_tui_tx.send(TuiEvent::TextDelta(
            "\nWorld model guardrail: required verification is missing; starting a repair turn before this can be marked complete.\n".into(),
        ));
        match dispatcher.lock().unwrap().spawn_turn(
            repair_prompt,
            adapter.clone() as Arc<dyn archon_tui::TurnRunner>,
        ) {
            archon_tui::DispatchResult::Running { .. } => {
                tracing::debug!("spawned guardrail repair turn");
            }
            archon_tui::DispatchResult::Queued => {
                tracing::debug!("queued guardrail repair turn");
            }
            archon_tui::DispatchResult::Rejected(error) => {
                tracing::error!("guardrail repair dispatch rejected: {error}");
            }
        }
        queue.push_back(PostTurnAction::PersistSession {
            guardrail: Some(guardrail),
        });
    }
}
