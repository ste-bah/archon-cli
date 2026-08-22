use std::collections::VecDeque;
use std::str::FromStr;
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
    // Mentions resolve here, before anything about this turn is announced.
    // A reference that cannot be read must stop the turn, and stopping it
    // before `GenerationStarted` means the UI never enters a generating state
    // it would then have to be talked out of. See `mention_resolve` for why
    // the snapshot is taken now rather than when the user picked the session.
    let mention_references =
        match super::mention_resolve::resolve_prompt_mentions(&input, cmd_ctx).await {
            Ok(references) => references,
            Err(reason) => {
                let _ = input_tui_tx.send_async(TuiEvent::Error(reason)).await;
                return;
            }
        };
    if !notify_generation_started(input_tui_tx).await {
        return;
    }
    let guardrail = match begin_prompt_guardrail(config, session_id, &input) {
        Ok(guardrail) => guardrail,
        Err(error) => {
            tracing::error!(%error, "world-model guardrail admission failed");
            let _ = input_tui_tx
                .send_async(TuiEvent::Error(format!(
                    "World model guardrail admission failed: {error}"
                )))
                .await;
            return;
        }
    };
    if let Some(record) = &guardrail
        && !record.decision.allowed_to_finalize
        && !record.decision.required_actions.is_empty()
        && let Err(error) = input_tui_tx
            .send_async(TuiEvent::TextDelta(format!(
                "\nWorld model guardrail: {:?} risk; verification required before completion: {:?}.\n",
                record.decision.risk_tier, record.decision.required_actions
            )))
            .await
    {
        tracing::error!(%error, "world-model guardrail notification delivery failed");
        crate::command::world_model::record_guardrail_turn_outcome(config, record, false);
        return;
    }

    let turn_runner: Arc<dyn archon_tui::TurnRunner> = guardrail
        .as_ref()
        .map(|record| adapter.scoped_turn_runner(record.action.action_id.clone()))
        .unwrap_or_else(|| adapter.clone());

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
    let current_mode = current_permission_mode(cmd_ctx).await;
    let mut session_references = drain_pending_session_references(cmd_ctx).await;
    session_references.extend(mention_references);
    let effective_input = compose_turn_input(
        input,
        initial_prompt_pending,
        session_references,
        current_mode,
    );
    let dispatch =
        dispatch_turn_after_generation_started(dispatcher, effective_input, turn_runner).await;
    if dispatch.is_none() {
        return;
    }
    queue.push_back(PostTurnAction::PersistSession {
        guardrail: guardrail.map(Box::new),
    });
}

async fn notify_generation_started(tui_tx: &archon_tui::event_channel::TuiEventSender) -> bool {
    if let Err(error) = tui_tx.send_async(TuiEvent::GenerationStarted).await {
        tracing::error!(%error, "generation-start notification delivery failed");
        return false;
    }
    true
}

async fn current_permission_mode(
    cmd_ctx: &SlashCommandContext,
) -> archon_permissions::mode::PermissionMode {
    let mode = cmd_ctx.permission_mode.lock().await;
    archon_permissions::mode::PermissionMode::from_str(&mode).unwrap_or_else(|error| {
        tracing::warn!(%error, mode = %mode, "invalid interactive permission mode; using default");
        archon_permissions::mode::PermissionMode::Default
    })
}

/// Take whatever `/session-ref` prepared, leaving the slot empty.
///
/// A prepared reference rides exactly one turn. Draining here rather than
/// re-reading a persistent list is what keeps a one-off "look at what the
/// other session found" from becoming a permanent tax on every later request.
async fn drain_pending_session_references(cmd_ctx: &SlashCommandContext) -> Vec<String> {
    std::mem::take(&mut *cmd_ctx.pending_session_references.lock().await)
}

/// Assemble the text the model actually receives.
///
/// `pub(super)` so the mention tests can assert on the real composition
/// rather than on an approximation of it.
pub(super) fn compose_turn_input(
    input: String,
    initial_prompt_pending: &mut Option<String>,
    session_references: Vec<String>,
    current_mode: archon_permissions::mode::PermissionMode,
) -> String {
    let input = if let Some(prefix) = initial_prompt_pending.take() {
        format!("{prefix}\n\n{input}")
    } else {
        input
    };
    // References go first and the user's own words last, so the closing
    // instruction the model reads is the one that actually carries authority.
    // Each block already carries its own untrusted wrapper from
    // `archon_core::session_reference`; nothing here unwraps or reformats it.
    let input = if session_references.is_empty() {
        input
    } else {
        format!("{}\n\n{input}", session_references.join("\n\n"))
    };
    crate::session::plan_hint::inject_plan_mode_hint(input, current_mode)
}

async fn dispatch_turn_after_generation_started(
    dispatcher: &Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>,
    prompt: String,
    runner: Arc<dyn archon_tui::TurnRunner>,
) -> Option<archon_tui::DispatchResult> {
    let result = dispatcher.lock().unwrap().spawn_turn(prompt, runner);
    match &result {
        archon_tui::DispatchResult::Running { .. } => tracing::debug!("spawned agent turn"),
        archon_tui::DispatchResult::Queued => tracing::debug!("agent busy; queued prompt"),
        archon_tui::DispatchResult::Rejected(error) => {
            tracing::error!("dispatch rejected: {error}");
        }
    }
    Some(result)
}

fn begin_prompt_guardrail(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    input: &str,
) -> anyhow::Result<Option<crate::command::world_model::RuntimeGuardrailRecord>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct NoopRouter;

    impl archon_tui::AgentRouter for NoopRouter {
        fn switch(&self, _agent_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    struct CountingRunner(AtomicUsize);

    impl archon_tui::TurnRunner for CountingRunner {
        fn run_turn<'a>(
            &'a self,
            _prompt: String,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }
    }

    struct RecordingRunner(Arc<std::sync::Mutex<Vec<String>>>);

    impl archon_tui::TurnRunner for RecordingRunner {
        fn run_turn<'a>(
            &'a self,
            prompt: String,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            self.0.lock().expect("prompt capture").push(prompt);
            Box::pin(async { Ok(()) })
        }
    }

    #[test]
    fn generation_started_uses_async_backpressure() {
        let source = include_str!("prompt_turn.rs");

        assert!(source.contains("async fn notify_generation_started"));
        assert!(source.contains("send_async(TuiEvent::GenerationStarted).await"));
    }

    #[test]
    fn guardrail_persistence_follows_generation_acceptance() {
        let source = include_str!("prompt_turn.rs");
        let body = source
            .split("pub(super) async fn dispatch_user_prompt")
            .nth(1)
            .expect("dispatch function");
        let notification = body
            .find("notify_generation_started(input_tui_tx)")
            .expect("generation notification");
        let guardrail = body
            .find("begin_prompt_guardrail(config, session_id, &input)")
            .expect("guardrail creation");

        assert!(notification < guardrail);
    }

    #[tokio::test]
    async fn dispatch_boundary_forwards_plan_hint_to_turn_runner() {
        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        let runner = Arc::new(RecordingRunner(captured.clone()));
        let (agent_event_tx, _agent_event_rx) = tokio::sync::mpsc::channel(1);
        let dispatcher = Arc::new(std::sync::Mutex::new(archon_tui::AgentDispatcher::new(
            Arc::new(NoopRouter),
            agent_event_tx,
        )));

        let mut initial_prompt = None;
        dispatch_turn_after_generation_started(
            &dispatcher,
            compose_turn_input(
                "Update src/a.rs and src/b.rs".into(),
                &mut initial_prompt,
                Vec::new(),
                archon_permissions::mode::PermissionMode::Default,
            ),
            runner,
        )
        .await;
        tokio::task::yield_now().await;

        // Scoped rather than dropped explicitly: a `MutexGuard` is not `Send`,
        // so one alive across an await point stops the enclosing future being
        // spawned at all. A block makes that impossible to reintroduce by
        // adding a line above the `drop`.
        {
            let prompts = captured.lock().expect("prompt capture");
            assert_eq!(prompts.len(), 1);
            assert!(prompts[0].starts_with("This request spans multiple implementation concerns."));
            assert!(prompts[0].ends_with("Update src/a.rs and src/b.rs"));
        }

        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        let runner = Arc::new(RecordingRunner(captured.clone()));
        let (agent_event_tx, _agent_event_rx) = tokio::sync::mpsc::channel(1);
        let dispatcher = Arc::new(std::sync::Mutex::new(archon_tui::AgentDispatcher::new(
            Arc::new(NoopRouter),
            agent_event_tx,
        )));
        let mut initial_prompt = None;
        dispatch_turn_after_generation_started(
            &dispatcher,
            compose_turn_input(
                "What update happened in src/a.rs and src/b.rs?".into(),
                &mut initial_prompt,
                Vec::new(),
                archon_permissions::mode::PermissionMode::Default,
            ),
            runner,
        )
        .await;
        tokio::task::yield_now().await;

        assert_eq!(
            captured.lock().expect("prompt capture").as_slice(),
            ["What update happened in src/a.rs and src/b.rs?"],
        );
    }

    /// #200 Phase 4. A prepared cross-session block must reach the prompt the
    /// turn actually runs, and it must lead: the user's own words come last so
    /// the closing instruction is the one that carries authority.
    #[test]
    fn session_references_lead_the_composed_turn() {
        let block = "<referenced-session-abc>quoted transcript</referenced-session-abc>";
        let mut initial_prompt = None;
        let composed = compose_turn_input(
            "summarise what that session concluded".into(),
            &mut initial_prompt,
            vec![block.to_string()],
            archon_permissions::mode::PermissionMode::Default,
        );

        assert!(
            composed.starts_with(block),
            "reference did not lead: {composed}"
        );
        assert!(composed.ends_with("summarise what that session concluded"));
    }

    /// The slot is emptied as it is read, so a block rides one turn. The same
    /// reference silently re-attached to every later request is the leak this
    /// pins against.
    #[tokio::test]
    async fn draining_the_reference_slot_empties_it() {
        let fixture = crate::command::context::slash_ctx_test_fixture::build_test_slash_context(
            "current-session",
            "default",
            None,
            None,
        );
        fixture
            .ctx
            .pending_session_references
            .lock()
            .await
            .push("<referenced-session-abc>x</referenced-session-abc>".to_string());

        assert_eq!(
            drain_pending_session_references(&fixture.ctx).await.len(),
            1
        );
        assert!(
            drain_pending_session_references(&fixture.ctx)
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn full_tui_prevents_agent_turn_launch_without_waiting() {
        let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
        tui_tx.send(TuiEvent::Done).expect("fill TUI event channel");
        let (agent_event_tx, _agent_event_rx) = tokio::sync::mpsc::channel(1);
        let dispatcher = Arc::new(std::sync::Mutex::new(archon_tui::AgentDispatcher::new(
            Arc::new(NoopRouter),
            agent_event_tx,
        )));
        let runner = Arc::new(CountingRunner(AtomicUsize::new(0)));

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(25),
            notify_generation_started(&tui_tx),
        )
        .await;

        assert!(
            result.is_err(),
            "full queue must backpressure prompt launch"
        );
        assert!(
            dispatcher
                .lock()
                .expect("dispatcher")
                .current_query
                .is_none()
        );
        assert_eq!(runner.0.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn closed_tui_prevents_agent_turn_launch() {
        let (tui_tx, rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
        drop(rx);
        let (agent_event_tx, _agent_event_rx) = tokio::sync::mpsc::channel(1);
        let dispatcher = Arc::new(std::sync::Mutex::new(archon_tui::AgentDispatcher::new(
            Arc::new(NoopRouter),
            agent_event_tx,
        )));
        let runner = Arc::new(CountingRunner(AtomicUsize::new(0)));

        let result = if notify_generation_started(&tui_tx).await {
            dispatch_turn_after_generation_started(
                &dispatcher,
                "must not launch".into(),
                runner.clone(),
            )
            .await
        } else {
            None
        };

        assert!(result.is_none());
        assert!(
            dispatcher
                .lock()
                .expect("dispatcher")
                .current_query
                .is_none()
        );
        assert_eq!(runner.0.load(Ordering::SeqCst), 0);
    }
}
