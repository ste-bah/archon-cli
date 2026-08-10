use std::sync::{Arc, atomic};

use anyhow::Result;
use archon_core::env_vars::ArchonEnvVars;
use archon_tui::event_channel::TuiEventReceiver;
use archon_tui::observability;
use archon_tui::{AgentDispatcher, app::TuiEvent};

use crate::cli_args::Cli;

#[path = "web_runtime_reply.rs"]
mod reply;
use reply::{auth_label, finish_reply};

const WEB_TURN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(86400);
const WEB_SESSION_SHUTDOWN_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(if cfg!(test) { 10 } else { 30_000 });
const WEB_SESSION_ABORT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(if cfg!(test) { 10 } else { 1_000 });

pub(crate) struct WebSessionHandle {
    input_tx: tokio::sync::Mutex<Option<tokio::sync::mpsc::Sender<String>>>,
    permission_tx: tokio::sync::mpsc::Sender<bool>,
    ask_user_tx: tokio::sync::mpsc::Sender<String>,
    event_rx: tokio::sync::Mutex<TuiEventReceiver>,
    last_assistant_response: Arc<tokio::sync::Mutex<String>>,
    cancel_handle: Arc<std::sync::Mutex<Option<Arc<crate::agent_handle::AgentHandle>>>>,
    loop_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<anyhow::Result<()>>>>,
    sandbox_audit_drain: crate::runtime::sandbox_audit_writer::SandboxAuditDrainHandle,
    dispatcher: Arc<std::sync::Mutex<AgentDispatcher>>,
    shutdown: tokio_util::sync::CancellationToken,
    leann_init_cancel: Arc<atomic::AtomicBool>,
}

impl WebSessionHandle {
    pub(crate) async fn submit(&self, input: String) -> Result<String> {
        let mut event_rx = self.event_rx.lock().await;
        while event_rx.try_recv().is_ok() {}

        let input_tx = self
            .input_tx
            .lock()
            .await
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("web session shut down"))?;
        tokio::select! {
            result = input_tx.send(input) => result
                .map_err(|_| anyhow::anyhow!("web session input channel closed"))?,
            _ = self.shutdown.cancelled() => anyhow::bail!("web session shut down"),
        }

        let mut reply = String::new();
        let mut timeout = Box::pin(tokio::time::sleep(WEB_TURN_TIMEOUT));
        loop {
            tokio::select! {
                event = event_rx.recv() => {
                    let Some(event) = event else {
                        anyhow::bail!("web session event channel closed");
                    };
                    if self.handle_event(event, &mut reply).await? {
                        let fallback = self.last_assistant_response.lock().await;
                        return Ok(finish_reply(&reply, &fallback));
                    }
                }
                _ = &mut timeout => {
                    self.cancel_inflight_turn();
                    anyhow::bail!(
                        "web session turn timed out after {}s",
                        WEB_TURN_TIMEOUT.as_secs()
                    );
                }
            }
        }
    }

    pub(crate) async fn begin_shutdown(&self) {
        // Stop the background repository index, as `interactive_ui` does for the
        // TUI. It is a `spawn_blocking` task and dropping a `#[tokio::main]`
        // runtime waits for those, so left unset Ctrl-C tears the HTTP server
        // down and then burns cores on ONNX embedding until the repo is indexed.
        self.leann_init_cancel
            .store(true, atomic::Ordering::Relaxed);
        self.shutdown.cancel();
        self.signal_inflight_turn();
        self.input_tx.lock().await.take();
    }

    pub(crate) async fn finish_shutdown(&self) -> Result<()> {
        let loop_result = match self.loop_task.lock().await.take() {
            Some(mut task) => {
                match tokio::time::timeout(WEB_SESSION_SHUTDOWN_TIMEOUT, &mut task).await {
                    Ok(result) => result
                        .map_err(|error| anyhow::anyhow!("web session loop task failed: {error}")),
                    Err(_) => {
                        task.abort();
                        let abort_result =
                            tokio::time::timeout(WEB_SESSION_ABORT_TIMEOUT, &mut task)
                                .await
                                .map_err(|_| {
                                    anyhow::anyhow!(
                                        "web session loop remained active {:?} after abort",
                                        WEB_SESSION_ABORT_TIMEOUT
                                    )
                                });
                        let suffix = abort_result
                            .err()
                            .map(|error| format!("; {error:#}"))
                            .unwrap_or_default();
                        Err(anyhow::anyhow!(
                            "web session loop shutdown timed out after {:?}{suffix}",
                            WEB_SESSION_SHUTDOWN_TIMEOUT
                        ))
                    }
                }
            }
            None => Ok(Ok(())),
        };
        let fallback_result = self
            .sandbox_audit_drain
            .shutdown(std::time::Duration::from_secs(30))
            .await;
        super::finish_loop_and_audit(loop_result, fallback_result)
    }

    fn signal_inflight_turn(&self) {
        if let Ok(guard) = self.cancel_handle.lock()
            && let Some(ref handle) = *guard
        {
            handle.fire_cancel();
        }
    }

    fn cancel_inflight_turn(&self) {
        self.signal_inflight_turn();
        if let Ok(mut dispatcher) = self.dispatcher.lock() {
            let _ = dispatcher.cancel_current();
        }
    }

    async fn handle_event(&self, event: TuiEvent, reply: &mut String) -> Result<bool> {
        match event {
            TuiEvent::TextDelta(text) | TuiEvent::BtwResponse(text) => reply.push_str(&text),
            TuiEvent::ToolStart { .. } | TuiEvent::ToolComplete { .. } => {}
            TuiEvent::TurnComplete { .. } | TuiEvent::SlashCommandComplete => return Ok(true),
            TuiEvent::Error(message) => {
                if reply.trim().is_empty() {
                    anyhow::bail!(message);
                }
                reply.push_str(&format!("\n{message}\n"));
                return Ok(true);
            }
            TuiEvent::PermissionPrompt { tool, description } => {
                let _ = self.permission_tx.send(false).await;
                anyhow::bail!(
                    "permission required for {tool}: {description}. \
                     Change permissions in the TUI or config before retrying from web chat"
                );
            }
            TuiEvent::AskUserPrompt { question } => {
                let _ = self.ask_user_tx.send(String::new()).await;
                anyhow::bail!(
                    "AskUserQuestion requires an interactive TUI answer: {question}. \
                     Retry this task in the TUI so Archon can ask for input."
                );
            }
            TuiEvent::Done => return Ok(true),
            _ => {}
        }
        Ok(false)
    }
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn spawn_web_session(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    env_vars: &ArchonEnvVars,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
) -> Result<Arc<WebSessionHandle>> {
    let super::interactive_bootstrap::Bootstrap {
        config_path,
        layer_filter,
        session_store,
        memory,
        working_dir,
        hook_registry,
        mcp_manager,
        mcp_tools,
        provider_override,
        anthropic_client,
        session_api_url,
        prompt_identity,
        fast_mode_shared,
        sandbox_flag,
        fast_mode,
        effort_state,
        effort_level_shared,
        model_override_shared,
        cost_alert_state,
        checkpoint_store,
    } = super::interactive_bootstrap::prepare(config, session_id, cli, env_vars, resolved_flags)
        .await?;

    let super::interactive_setup::Setup {
        registry,
        agent_def,
        active_model,
        permission_mode_shared,
        btw_system_prompt: _,
        system_prompt_chars,
        tool_defs_chars,
        agent_config,
        cron_shutdown: _,
    } = super::interactive_setup::prepare(
        config,
        session_id,
        cli,
        resolved_flags,
        Arc::clone(&session_store),
        Arc::clone(&memory),
        working_dir.clone(),
        prompt_identity,
        mcp_tools,
        Arc::clone(&fast_mode_shared),
        Arc::clone(&effort_level_shared),
        Arc::clone(&model_override_shared),
        Arc::clone(&sandbox_flag),
    )
    .await?;

    let agent_model_for_ledger = agent_config.model.clone();
    let extra_dirs_shared = Arc::clone(&agent_config.extra_dirs);

    let super::interactive_agent::Runtime {
        mut agent,
        provider,
        agent_event_rx,
        tui_event_tx,
        tui_event_rx,
        user_input_tx,
        user_input_rx,
        agent_registry_for_skills,
        task_service,
        coding_pipeline,
        research_pipeline,
        llm_adapter,
        leann,
        leann_init_cancel,
        learning_cozo_db,
        governed_learning_db,
        auto_trainer,
        metrics,
        agent_event_tx_for_dispatcher,
        sandbox_audit_drain,
    } = super::interactive_agent::build(
        config,
        session_id,
        cli,
        working_dir.clone(),
        Arc::clone(&hook_registry),
        provider_override,
        anthropic_client,
        Arc::clone(&memory),
        Arc::clone(&session_store),
        checkpoint_store,
        agent_config,
        registry,
        None,
    )
    .await?;

    let auto_capture = if config.memory.auto_capture.enabled && config.memory.enabled {
        Some(Arc::new(archon_pipeline::capture::AutoCapture::new(true)))
    } else {
        None
    };

    let super::interactive_finish::FinishState {
        perm_prompt_tx,
        ask_user_tx,
        show_thinking,
        session_stats_shared,
        last_assistant_response_shared,
        active_session,
        // The web runtime has no splash to surface it in.
        garden_summary: _,
    } = super::interactive_finish::finish(
        &mut agent,
        config,
        session_id,
        cli,
        config_path.clone(),
        working_dir.clone(),
        Arc::clone(&memory),
        Arc::clone(&llm_adapter),
        Arc::clone(&hook_registry),
        governed_learning_db.clone(),
        Arc::clone(&session_store),
        tui_event_tx.clone(),
        agent_event_rx,
        Arc::clone(&metrics),
        cost_alert_state,
        Arc::clone(&permission_mode_shared),
        agent_def.as_ref(),
        agent_model_for_ledger,
        provider.name().to_string(),
        None,
    )
    .await;

    let auth_label = auth_label(env_vars);
    let dispatcher: Arc<std::sync::Mutex<archon_tui::AgentDispatcher>> =
        Arc::new(std::sync::Mutex::new(archon_tui::AgentDispatcher::new(
            Arc::new(crate::agent_handle::NoopAgentRouter),
            agent_event_tx_for_dispatcher,
        )));
    let cancel_handle: Arc<std::sync::Mutex<Option<Arc<crate::agent_handle::AgentHandle>>>> =
        Arc::new(std::sync::Mutex::new(None));
    let context_override = config
        .context
        .context_window_override
        .or_else(|| config.context.max_tokens.map(u64::from));
    let context_resolution = archon_llm::context_window::resolve_context_window_for_work_dir(
        &active_model,
        context_override,
        Some(provider.as_ref()),
        Some(&working_dir),
    );

    let cmd_ctx =
        super::slash_context_builder::build(super::slash_context_builder::SlashContextBuildInput {
            fast_mode_shared,
            effort_level_shared,
            model_override_shared,
            default_model: active_model.clone(),
            context_window: context_resolution.context_window,
            context_source: context_resolution.source.label().to_string(),
            show_thinking,
            session_stats: session_stats_shared,
            permission_mode: permission_mode_shared,
            session_id: session_id.to_string(),
            session_store: Arc::clone(&session_store),
            cost_config: config.cost.clone(),
            codex_models: config.models.openai_codex.clone(),
            anthropic_models: config.models.anthropic.clone(),
            memory: Arc::clone(&memory),
            garden_config: config.memory.garden.clone(),
            mcp_manager: mcp_manager.clone(),
            working_dir: working_dir.clone(),
            extra_dirs: extra_dirs_shared,
            auth_label,
            config_path,
            env_vars: env_vars.clone(),
            cli_settings: cli.settings.clone(),
            layer_filter,
            last_assistant_response: Arc::clone(&last_assistant_response_shared),
            system_prompt_chars,
            tool_defs_chars,
            allow_bypass_permissions: cli.allow_dangerously_skip_permissions
                || cli.dangerously_skip_permissions,
            denial_log: Arc::clone(&agent.denial_log),
            agent_registry: agent_registry_for_skills,
            task_service,
            coding_pipeline,
            research_pipeline,
            llm_adapter,
            leann,
            sandbox_flag,
            hook_registry,
            cancel_handle: Arc::clone(&cancel_handle),
            agent_dispatcher: Arc::clone(&dispatcher),
            cozo_db: learning_cozo_db,
            governed_learning_db,
            auto_trainer: auto_trainer.clone(),
        });

    let mcp_lifecycle_tx = crate::session_loop::spawn_mcp_lifecycle_task(mcp_manager);
    super::cognitive_daemon_startup::ensure_for_session(config, &working_dir, &tui_event_tx);
    let persist_personality = config.consciousness.persist_personality;
    let personality_history_limit = config.consciousness.personality_history_limit;
    let session_start_instant = std::time::Instant::now();
    let session_start_confidence = if let Some(iv_arc) = agent.inner_voice() {
        iv_arc.lock().await.confidence
    } else {
        0.7
    };

    if let Some(iv_arc) = agent.inner_voice().cloned() {
        let initial_iv = iv_arc.lock().await.clone();
        let mirror = crate::panic_save::install(
            Arc::clone(&cmd_ctx.memory),
            initial_iv,
            cmd_ctx.session_id.clone(),
            session_start_confidence,
            session_start_instant,
            personality_history_limit,
        );
        let mirror_for_cb = Arc::clone(&mirror);
        let cb: Arc<dyn Fn(&archon_consciousness::inner_voice::InnerVoice) + Send + Sync> =
            Arc::new(move |new_state| {
                let snapshot = new_state.clone();
                match mirror_for_cb.lock() {
                    Ok(mut m) => *m = snapshot,
                    Err(p) => *p.into_inner() = snapshot,
                }
            });
        agent.set_inner_voice_change_callback(cb);
    }

    let sandbox_audit_drain =
        crate::runtime::sandbox_audit_writer::SandboxAuditDrainHandle::new(sandbox_audit_drain);
    let shutdown = tokio_util::sync::CancellationToken::new();
    let loop_task = observability::spawn_named(
        "web-session-loop",
        crate::session_loop::run_session_loop(
            agent,
            config.clone(),
            agent_def,
            session_api_url,
            tui_event_tx,
            user_input_rx,
            Arc::clone(&session_store),
            active_session,
            persist_personality,
            personality_history_limit,
            session_start_instant,
            session_start_confidence,
            resolved_flags.disable_slash_commands,
            fast_mode,
            effort_state,
            cmd_ctx,
            mcp_lifecycle_tx,
            auto_capture,
            auto_trainer,
            Arc::clone(&dispatcher),
            Arc::clone(&cancel_handle),
            sandbox_audit_drain.clone(),
            false,
            shutdown.clone(),
        ),
    );

    Ok(Arc::new(WebSessionHandle {
        input_tx: tokio::sync::Mutex::new(Some(user_input_tx)),
        permission_tx: perm_prompt_tx,
        ask_user_tx,
        event_rx: tokio::sync::Mutex::new(tui_event_rx),
        last_assistant_response: last_assistant_response_shared,
        cancel_handle,
        loop_task: tokio::sync::Mutex::new(Some(loop_task)),
        sandbox_audit_drain,
        dispatcher,
        shutdown,
        leann_init_cancel,
    }))
}

#[cfg(test)]
#[path = "web_runtime_tests.rs"]
mod tests;
