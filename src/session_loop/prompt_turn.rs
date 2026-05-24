use std::collections::VecDeque;
use std::sync::Arc;

use archon_core::agent::Agent;
use archon_tui::app::TuiEvent;

use super::post_turn::PostTurnAction;
use crate::slash_context::SlashCommandContext;

#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch_user_prompt(
    input: String,
    initial_prompt_pending: &mut Option<String>,
    queue: &mut VecDeque<PostTurnAction>,
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    config: &archon_core::config::ArchonConfig,
    input_tui_tx: &archon_tui::event_channel::TuiEventSender,
    cmd_ctx: &SlashCommandContext,
    session_id: &str,
    dispatcher: &Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>,
    adapter: &Arc<crate::agent_handle::AgentHandle>,
) {
    {
        let mut response = cmd_ctx.last_assistant_response.lock().await;
        response.clear();
    }
    {
        let guard = agent.lock().await;
        guard
            .fire_hook_detached(
                archon_core::hooks::HookType::UserPromptSubmit,
                serde_json::json!({
                    "hook_event": "UserPromptSubmit",
                    "prompt_length": input.len(),
                }),
            )
            .await;
    }

    let _ = input_tui_tx.send(TuiEvent::GenerationStarted);
    let effective_input = if let Some(prefix) = initial_prompt_pending.take() {
        format!("{prefix}\n\n{input}")
    } else {
        input.clone()
    };
    let guardrail = begin_prompt_guardrail(config, session_id, &input);
    if let Some(record) = &guardrail
        && !record.decision.allowed_to_finalize
        && !record.decision.required_actions.is_empty()
    {
        let _ = input_tui_tx.send(TuiEvent::TextDelta(format!(
            "\nWorld model guardrail: {:?} risk; verification required before completion: {:?}.\n",
            record.decision.risk_tier, record.decision.required_actions
        )));
    }

    match dispatcher.lock().unwrap().spawn_turn(
        effective_input,
        adapter.clone() as Arc<dyn archon_tui::TurnRunner>,
    ) {
        archon_tui::DispatchResult::Running { .. } => {
            tracing::debug!("spawned agent turn");
        }
        archon_tui::DispatchResult::Queued => {
            tracing::debug!("agent busy; queued prompt");
        }
        archon_tui::DispatchResult::Rejected(error) => {
            tracing::error!("dispatch rejected: {error}");
        }
    }
    queue.push_back(PostTurnAction::PersistSession { guardrail });
}

fn begin_prompt_guardrail(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    input: &str,
) -> Option<crate::command::world_model::RuntimeGuardrailRecord> {
    let task_class = archon_world_model::guardrail::classify_task(
        input,
        archon_world_model::integration::WorldAdvisorSurface::InteractiveSession,
    );
    let guardrail_surface = match task_class {
        archon_world_model::RuntimeTaskClass::CodingChange
        | archon_world_model::RuntimeTaskClass::Debugging
        | archon_world_model::RuntimeTaskClass::Refactor => {
            archon_world_model::integration::WorldAdvisorSurface::CodingTask
        }
        _ => archon_world_model::integration::WorldAdvisorSurface::InteractiveSession,
    };
    let action_ref = format!("interactive-turn-{}", uuid::Uuid::new_v4());
    crate::command::world_model::begin_guarded_action(
        config,
        guardrail_surface,
        session_id,
        &action_ref,
        input,
    )
}
