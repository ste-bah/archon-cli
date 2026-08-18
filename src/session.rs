//! Print-mode session runner. Extracted from main.rs.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::cli_args::Cli;
pub(crate) use crate::command::utils::{
    handle_resume_list_with_config, load_resume_messages_with_config,
};
use anyhow::Result;
use archon_core::agent::{Agent, TimestampedEvent};
use archon_core::env_vars::ArchonEnvVars;
use archon_tui::observability;

use crate::runtime::provider_observer::observe_llm_provider_with_profile;
pub(crate) mod active_session;
mod activity;
mod agent_ledger;
mod btw;
pub(crate) mod build_agent;
mod build_prompt;
mod cognitive_daemon_startup;
mod cognitive_store;
mod command_catalog;
mod completion_gate;
mod config_watcher;
mod consolidation_reuse;
mod event_forwarder;
mod garden_rule_observations;
mod garden_scheduler;
mod gnn_auto_trainer_seed;
mod interactive_agent;
mod interactive_bootstrap;
mod interactive_finish;
mod interactive_learning_init;
#[cfg(test)]
mod interactive_learning_init_tests;
mod interactive_setup;
mod interactive_ui;
mod leann_startup;
mod model_selection;
mod modes;
mod pipeline_adapter;
pub(crate) mod plan_hint;
mod reasoning_quality;
mod slash_context_builder;
mod splash;
mod task_overlay_store;
mod web_runtime;
mod world_model_backend;
mod world_model_callbacks;
use activity::{session_activity_sink, session_activity_sink_with_tui};
pub(super) use model_selection::active_session_model;
pub(crate) use modes::{run_headless_session, run_print_mode_session};
pub(crate) use web_runtime::{WebSessionHandle, spawn_web_session};

#[cfg(test)]
fn anthropic_model_env_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

/// Result of [`build_agent::build_session_agent`] — a fully constructed Agent
/// plus the event receiver, resolved agent definition, and channel metrics.
#[allow(dead_code)]
pub(crate) struct BuiltAgent {
    pub(crate) agent: Agent,
    pub(crate) event_rx: tokio::sync::mpsc::Receiver<TimestampedEvent>,
    pub(crate) agent_def: Option<archon_core::agents::definition::CustomAgentDefinition>,
    pub(crate) metrics: std::sync::Arc<archon_tui::observability::ChannelMetrics>,
    pub(crate) selected_provider: String,
    pub(crate) selected_model: String,
    pub(crate) permission_mode: Arc<tokio::sync::Mutex<String>>,
    pub(crate) sandbox_audit_drain: crate::runtime::sandbox_audit_writer::SandboxAuditDrain,
}

pub(super) fn is_codex_session(config: &archon_core::config::ArchonConfig) -> bool {
    config.llm.provider == "openai-codex"
}

async fn native_session_sandbox_backend(
    config: &archon_core::config::ArchonConfig,
    sandbox_flag: Arc<AtomicBool>,
) -> Arc<dyn archon_permissions::SandboxBackend> {
    let backend: Arc<dyn archon_permissions::SandboxBackend> = match config.sandbox.backend.as_str()
    {
        "docker" => Arc::new(archon_core::sandbox::DockerSandboxBackend::new(
            config.sandbox.docker.clone(),
            config.sandbox.workspace_access.clone(),
        )),
        "ssh" => Arc::new(archon_core::sandbox::SshSandboxBackend::new(
            config.sandbox.ssh.clone(),
        )),
        "openshell" => Arc::new(archon_core::sandbox::OpenShellSandboxBackend::new(
            config.sandbox.openshell.clone(),
        )),
        _ => Arc::new(archon_tui::sandbox::SharedSandboxFlag::with_flag(
            sandbox_flag,
        )),
    };
    crate::runtime::sandbox_mode::apply_configured_sandbox_mode(backend, &config.sandbox)
}

fn finish_loop_and_audit(
    loop_result: anyhow::Result<anyhow::Result<()>>,
    audit_result: anyhow::Result<
        Option<crate::runtime::sandbox_audit_writer::SandboxAuditReadback>,
    >,
) -> anyhow::Result<()> {
    match (loop_result.and_then(|result| result), audit_result) {
        (Ok(()), Ok(_)) => Ok(()),
        (Err(loop_error), Ok(_)) => Err(loop_error),
        (Ok(()), Err(audit_error)) => Err(audit_error),
        (Err(loop_error), Err(audit_error)) => Err(anyhow::anyhow!(
            "session loop failed: {loop_error:#}; sandbox audit drain failed: {audit_error:#}"
        )),
    }
}

async fn drain_startup_sandbox_audit(
    drain: crate::runtime::sandbox_audit_writer::SandboxAuditDrain,
) -> anyhow::Result<crate::runtime::sandbox_audit_writer::SandboxAuditReadback> {
    drain.shutdown(std::time::Duration::from_secs(30)).await
}

fn finish_startup_failure(
    startup_error: anyhow::Error,
    audit_result: anyhow::Result<crate::runtime::sandbox_audit_writer::SandboxAuditReadback>,
) -> anyhow::Error {
    match audit_result {
        Ok(_) => startup_error,
        Err(audit_error) => anyhow::anyhow!(
            "startup failed: {startup_error:#}; sandbox audit drain failed: {audit_error:#}"
        ),
    }
}

async fn open_governed_learning_db(working_dir: &std::path::Path) -> Option<Arc<cozo::DbInstance>> {
    match crate::runtime::learning_store::acquire_for_dir_async(working_dir).await {
        Ok(db) => Some(db),
        Err(error) => {
            tracing::warn!(
                %error,
                "governed learning store unavailable; runtime evidence disabled"
            );
            None
        }
    }
}

async fn open_cognitive_store(
    working_dir: &std::path::Path,
) -> anyhow::Result<Option<archon_cognitive::PersistentCognitiveStore>> {
    cognitive_store::open(working_dir).await
}

fn configure_session_vlm_provider(working_dir: &std::path::Path) {
    match archon_policy::load_effective_policy(working_dir) {
        Ok(policy) => {
            let report =
                archon_docs::vlm::factory::configure_registered_provider_thread_safe(&policy);
            match report.status {
                archon_docs::vlm::factory::VlmProviderInitStatus::Registered => tracing::info!(
                    provider = %report.provider,
                    model = %report.model,
                    "vlm provider registered for session"
                ),
                archon_docs::vlm::factory::VlmProviderInitStatus::Skipped => tracing::warn!(
                    provider = %report.provider,
                    model = %report.model,
                    reason = %report.message,
                    "vlm provider unavailable for session"
                ),
                archon_docs::vlm::factory::VlmProviderInitStatus::Disabled => tracing::debug!(
                    reason = %report.message,
                    "vlm provider disabled for session"
                ),
            }
        }
        Err(e) => {
            archon_docs::vlm::clear_provider();
            tracing::debug!(error = %e, "could not load VLM policy for session");
        }
    }
}

async fn build_codex_session_provider(
    config: &archon_core::config::ArchonConfig,
) -> Result<Arc<dyn archon_llm::provider::LlmProvider>> {
    let (provider, runtime_mode) =
        crate::runtime::codex_provider::build_codex_provider(config, "tui_session").await?;
    let profile_id =
        crate::runtime::provider_auth_selection::selected_provider_auth_profile_id_async(
            provider.name(),
        )
        .await;
    Ok(observe_llm_provider_with_profile(provider, runtime_mode, profile_id).await)
}

/// Spawn the Prometheus `/metrics` exporter when `--metrics-port PORT` is
/// both present and non-zero. Port 0 is treated as "disabled" per the
/// documented CLI contract (otherwise `--metrics-port 0` would bind to an
/// OS-chosen ephemeral port, which is useless for scraping).
///
/// Bind failures are validated synchronously: we call `TcpListener::bind`
/// *before* spawning the serve task so a "permission denied" / "address in
/// Construct the LEANN CodeIndex for the tool registry.
///
/// Resilient: returns `None` when the DB fails to open. The caller
/// propagates `None` through `create_default_registry`, which skips
/// LEANN tool registration — agent sees no LeannSearch/LeannFindSimilar
/// in ToolSearch results, graceful no-op.
fn init_leann_index(
    working_dir: &std::path::Path,
) -> Option<std::sync::Arc<archon_leann::CodeIndex>> {
    let db_path = working_dir.join(".archon").join("leann.db");
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match archon_leann::CodeIndex::new(&db_path, Default::default()) {
        Ok(idx) => Some(std::sync::Arc::new(idx)),
        Err(e) => {
            tracing::warn!(error = %e, "LEANN unavailable; continuing without code context");
            None
        }
    }
}

/// use" error propagates as `Err` to the caller rather than disappearing
/// into a `tokio::spawn` closure where the TUI swallows stderr. Post-bind
/// serve failures (peer reset, listener EOF) still warn-and-exit in the
/// background because the listener is live at that point.
fn spawn_metrics_exporter(
    port: Option<u16>,
    metrics: Arc<observability::ChannelMetrics>,
) -> Result<()> {
    let Some(port) = port else { return Ok(()) };
    if port == 0 {
        // Contract: 0 = disabled. Skip bind entirely.
        return Ok(());
    }
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = std::net::TcpListener::bind(addr)
        .map_err(|e| anyhow::anyhow!("--metrics-port {port}: bind failed: {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| anyhow::anyhow!("--metrics-port {port}: set_nonblocking failed: {e}"))?;
    // Hand the bound listener to tokio — this converts a std listener into a
    // tokio one so serve_metrics can accept connections from the runtime.
    let tokio_listener = tokio::net::TcpListener::from_std(listener)
        .map_err(|e| anyhow::anyhow!("--metrics-port {port}: tokio adapt failed: {e}"))?;
    observability::spawn_named("metrics-exporter", async move {
        if let Err(e) = observability::serve_metrics_on(tokio_listener, metrics).await {
            tracing::warn!(%e, port, "metrics exporter terminated");
        }
    });
    tracing::info!(port, "Prometheus /metrics exporter bound on 127.0.0.1");
    Ok(())
}

pub(crate) async fn run_interactive_session(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    env_vars: &ArchonEnvVars,
    resume_messages: Option<Vec<serde_json::Value>>,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    voice_event_rx: Option<tokio::sync::mpsc::Receiver<archon_tui::app::TuiEvent>>,
) -> Result<()> {
    let interactive_bootstrap::Bootstrap {
        config_path,
        layer_filter,
        session_database,
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
    } = interactive_bootstrap::prepare(config, session_id, cli, env_vars, resolved_flags).await?;

    let interactive_setup::Setup {
        registry,
        agent_def,
        active_model,
        permission_mode_shared,
        btw_system_prompt,
        system_prompt_chars,
        tool_defs_chars,
        agent_config,
        cron_shutdown,
    } = interactive_setup::prepare(
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

    let interactive_agent::Runtime {
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
    } = interactive_agent::build(
        config,
        session_id,
        session_database,
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
        voice_event_rx,
    )
    .await?;

    let auto_capture = if config.memory.auto_capture.enabled && config.memory.enabled {
        Some(Arc::new(archon_pipeline::capture::AutoCapture::new(true)))
    } else {
        None
    };

    let interactive_finish::FinishState {
        perm_prompt_tx,
        ask_user_tx,
        show_thinking,
        session_stats_shared,
        last_assistant_response_shared,
        active_session,
        garden_summary,
    } = interactive_finish::finish(
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
        resume_messages,
    )
    .await;

    let plan_mode_state = agent.plan_mode_state();

    interactive_ui::run(
        config,
        session_id,
        cli,
        env_vars,
        resolved_flags,
        config_path,
        layer_filter,
        working_dir,
        session_store,
        memory,
        agent,
        agent_def,
        session_api_url,
        provider,
        mcp_manager,
        cron_shutdown,
        fast_mode_shared,
        fast_mode,
        effort_level_shared,
        effort_state,
        model_override_shared,
        permission_mode_shared,
        plan_mode_state,
        extra_dirs_shared,
        show_thinking,
        session_stats_shared,
        last_assistant_response_shared,
        system_prompt_chars,
        tool_defs_chars,
        agent_registry_for_skills,
        task_service,
        coding_pipeline,
        research_pipeline,
        llm_adapter,
        leann,
        sandbox_flag,
        hook_registry,
        learning_cozo_db,
        governed_learning_db,
        auto_trainer,
        leann_init_cancel,
        agent_event_tx_for_dispatcher,
        tui_event_tx,
        tui_event_rx,
        user_input_tx,
        user_input_rx,
        perm_prompt_tx,
        ask_user_tx,
        btw_system_prompt,
        active_model,
        auto_capture,
        sandbox_audit_drain,
        active_session,
        garden_summary,
    )
    .await
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
