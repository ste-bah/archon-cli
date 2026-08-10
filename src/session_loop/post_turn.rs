use std::collections::VecDeque;
use std::sync::Arc;

use archon_core::agent::Agent;
use archon_tui::app::TuiEvent;

use crate::slash_context::SlashCommandContext;

pub(super) enum PostTurnAction {
    PersistSession {
        guardrail: Option<Box<crate::command::world_model::RuntimeGuardrailRecord>>,
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
    dispatcher: &Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>,
    adapter: &Arc<crate::agent_handle::AgentHandle>,
    cmd_ctx: &mut SlashCommandContext,
    active_session: &crate::session::active_session::ActiveSessionId,
) {
    match queue.pop_front() {
        Some(PostTurnAction::PersistSession { guardrail }) => {
            persist_session_messages(agent, session_store, active_session).await;
            if let Some(guardrail) = guardrail {
                let guardrail_outcome = maybe_spawn_guardrail_repair(
                    outcome,
                    config,
                    input_tui_tx,
                    dispatcher,
                    adapter,
                    queue,
                    *guardrail,
                )
                .await;
                record_turn_latent_surprise(
                    agent,
                    active_session,
                    &cmd_ctx.working_dir,
                    guardrail_outcome,
                )
                .await;
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
            if let Err(error) = input_tui_tx
                .send_async(TuiEvent::SlashCommandComplete)
                .await
            {
                tracing::warn!(%error, "skill command completion delivery failed");
            }
        }
        None => {}
    }
}

/// Write the conversation to the session row it belongs to.
///
/// Takes the shared id rather than the loop's process-scoped one: a resume
/// repoints this at the session the user picked, so continued work lands there
/// instead of in the row this process minted at launch.
async fn persist_session_messages(
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    session_store: &Arc<archon_session::storage::SessionStore>,
    active_session: &crate::session::active_session::ActiveSessionId,
) {
    let session_id = active_session.get();
    let session_id = session_id.as_str();
    let guard = agent.lock().await;
    let messages: Vec<String> = guard
        .conversation_state()
        .messages
        .iter()
        .filter_map(|msg| serde_json::to_string(msg).ok())
        .collect();
    drop(guard);
    if messages.is_empty() {
        // Previously silent. An empty conversation at turn end wrote nothing
        // and logged nothing, so a session that banked cost but no messages
        // left no evidence of why -- which is exactly the state a killed turn
        // leaves behind.
        tracing::warn!(
            session_id,
            "post-turn persist skipped: conversation is empty"
        );
        return;
    }
    if let Err(error) = session_store.replace_messages(session_id, &messages) {
        tracing::warn!(session_id, %error, "replace_messages post-turn failed");
    }
}

#[allow(clippy::too_many_arguments)]
async fn maybe_spawn_guardrail_repair(
    outcome: archon_tui::TurnOutcome,
    config: &archon_core::config::ArchonConfig,
    input_tui_tx: &archon_tui::event_channel::TuiEventSender,
    dispatcher: &Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>,
    adapter: &Arc<crate::agent_handle::AgentHandle>,
    queue: &mut VecDeque<PostTurnAction>,
    guardrail: crate::command::world_model::RuntimeGuardrailRecord,
) -> Option<archon_world_model::WorldGuardrailOutcome> {
    let (completed, spawn_repair) = turn_completion_state(&outcome);
    let guardrail_outcome =
        crate::command::world_model::record_guardrail_turn_outcome(config, &guardrail, completed);
    record_turn_world_model_outcome(config, &guardrail, &outcome);
    if spawn_repair
        && guardrail_outcome.as_ref().is_some_and(|outcome| {
            matches!(
                outcome.final_status,
                archon_world_model::GuardrailFinalStatus::BlockedMissingVerification
                    | archon_world_model::GuardrailFinalStatus::BlockedFailedVerification
            )
        })
        && let Some(repair_prompt) = crate::command::world_model::forced_repair_prompt(&guardrail)
    {
        if let Err(error) = input_tui_tx
            .send_async(TuiEvent::TextDelta(
                "\nWorld model guardrail: required verification is missing; starting a repair turn before this can be marked complete.\n".into(),
            ))
            .await
        {
            tracing::error!(%error, "guardrail repair notification delivery failed");
            return guardrail_outcome;
        }
        match dispatcher.lock().unwrap().spawn_turn(
            repair_prompt,
            adapter.scoped_turn_runner(guardrail.action.action_id.clone()),
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
            guardrail: Some(Box::new(guardrail)),
        });
    }
    guardrail_outcome
}

/// Record this turn's world-model surprise as a `surprise_observed` metric.
///
/// `latent_surprise_mean` and `latent_surprise_p95` are defined in
/// `archon-cognitive`'s R8 table and had no producer at all: the event kind
/// demands a prediction, an action attempt and a verification, and the
/// cognitive turn loop has no verification identity to give it. The guardrail
/// outcome does — the same action id the world-model corpus labels on, the
/// prediction the surprise was computed against, and the verification that
/// adjudicated the action.
///
/// Fails open, off the async runtime, and after the turn is already complete:
/// a measurement may degrade, but it may not cost the user a turn. An action
/// that cannot supply all three identities writes nothing rather than a row
/// with an invented one.
async fn record_turn_latent_surprise(
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    active_session: &crate::session::active_session::ActiveSessionId,
    working_dir: &std::path::Path,
    guardrail_outcome: Option<archon_world_model::WorldGuardrailOutcome>,
) {
    let Some(outcome) = guardrail_outcome else {
        return;
    };
    // The session the conversation was just persisted under, so a metric row
    // joins to the same session a reader would look the turn up in.
    let session_id = active_session.get().to_string();
    let guard = agent.lock().await;
    let turn_number = guard.turn_number();
    let model_id = guard.current_model().to_owned();
    drop(guard);
    let working_dir = working_dir.to_path_buf();
    let recorded =
        archon_observability::spawn_blocking_named("record-latent-surprise", move || {
            crate::command::world_model::record_latent_surprise(
                crate::command::world_model::LatentSurpriseContext {
                    working_dir: &working_dir,
                    session_id: &session_id,
                    turn_number,
                    model_id: &model_id,
                },
                &outcome,
            )
        })
        .await;
    match recorded {
        Ok(Ok(Some(written))) => tracing::debug!(?written, "latent surprise recorded"),
        Ok(Ok(None)) => tracing::debug!("turn had no verified prediction to measure surprise on"),
        Ok(Err(error)) => tracing::warn!(%error, "latent surprise metric write failed"),
        Err(error) => tracing::warn!(%error, "latent surprise metric task failed"),
    }
}

/// Close the world model's predict -> observe loop for an interactive turn.
///
/// `record_runtime_outcome` attaches the actual next-state summary to the
/// persisted prediction and computes latent surprise. Until now it fired only
/// on the pipeline surfaces, so interactive turns produced predictions that
/// were never scored against reality — which is the half of the loop that
/// makes the corpus trainable, and the gate on ever clearing `shadow_only`.
///
/// The advisory record is already threaded here on the guardrail record, so
/// this needs no extra plumbing through the turn.
fn record_turn_world_model_outcome(
    config: &archon_core::config::ArchonConfig,
    guardrail: &crate::command::world_model::RuntimeGuardrailRecord,
    outcome: &archon_tui::TurnOutcome,
) {
    crate::command::world_model::record_runtime_outcome(
        config,
        &guardrail.advisory,
        &turn_outcome_summary(outcome),
        Some(&guardrail.action.action_id),
    );
}

/// Summarise a turn outcome for the world model's actual-next-state field.
///
/// Deliberately records the failure modes distinctly rather than collapsing
/// them to "not completed": a turn blocked on verification and a turn the user
/// interrupted are different labels, and the labeler cannot recover the
/// difference later.
fn turn_outcome_summary(outcome: &archon_tui::TurnOutcome) -> String {
    match outcome {
        archon_tui::TurnOutcome::Completed => "interactive turn completed".to_string(),
        archon_tui::TurnOutcome::Cancelled => "interactive turn cancelled by user".to_string(),
        archon_tui::TurnOutcome::FinalizationBlocked(reason) => {
            format!("interactive turn blocked at finalization: {reason}")
        }
        archon_tui::TurnOutcome::Failed(reason) => {
            format!("interactive turn failed: {reason}")
        }
    }
}

fn turn_completion_state(outcome: &archon_tui::TurnOutcome) -> (bool, bool) {
    match outcome {
        archon_tui::TurnOutcome::Completed => (true, true),
        archon_tui::TurnOutcome::FinalizationBlocked(_) => (true, false),
        archon_tui::TurnOutcome::Cancelled | archon_tui::TurnOutcome::Failed(_) => (false, false),
    }
}

#[cfg(test)]
mod tests {
    use super::turn_completion_state;

    #[test]
    fn finalization_blocked_is_recorded_as_blocked_without_spawning_another_repair() {
        assert_eq!(
            turn_completion_state(&archon_tui::TurnOutcome::FinalizationBlocked(
                "run tests".into(),
            )),
            (true, false)
        );
    }
}
