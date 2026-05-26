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
mod activity;
mod agent_ledger;
mod btw;
mod build_agent;
mod build_prompt;
mod cognitive_daemon_startup;
mod config_watcher;
mod event_forwarder;
mod gnn_auto_trainer_seed;
mod interactive_agent;
mod interactive_bootstrap;
mod interactive_finish;
mod interactive_setup;
mod interactive_ui;
mod model_selection;
mod modes;
mod pipeline_adapter;
mod reasoning_quality;
mod slash_context_builder;
mod splash;
mod web_runtime;
use activity::{session_activity_sink, session_activity_sink_with_tui};
pub(super) use model_selection::active_session_model;
pub(crate) use modes::{run_headless_session, run_print_mode_session};
pub(crate) use web_runtime::{WebSessionHandle, spawn_web_session};

/// Result of [`build_session_agent`] — a fully constructed Agent plus
/// the event receiver, resolved agent definition, and channel metrics.
#[allow(dead_code)]
struct BuiltAgent {
    agent: Agent,
    event_rx: tokio::sync::mpsc::UnboundedReceiver<TimestampedEvent>,
    agent_def: Option<archon_core::agents::definition::CustomAgentDefinition>,
    metrics: std::sync::Arc<archon_tui::observability::ChannelMetrics>,
    selected_provider: String,
    selected_model: String,
    permission_mode: Arc<tokio::sync::Mutex<String>>,
}

pub(super) fn is_codex_session(config: &archon_core::config::ArchonConfig) -> bool {
    config.llm.provider == "openai-codex"
}

fn session_sandbox_backend(
    config: &archon_core::config::ArchonConfig,
    sandbox_flag: Arc<AtomicBool>,
    session_id: &str,
    agent_type: &str,
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
    let backend =
        crate::runtime::sandbox_mode::apply_configured_sandbox_mode(backend, &config.sandbox);
    crate::runtime::sandbox_audit::audit_sandbox_backend(backend, config, session_id, agent_type)
}

fn open_governed_learning_db(working_dir: &std::path::Path) -> Option<Arc<cozo::DbInstance>> {
    let db_path = std::env::var_os("ARCHON_LEARNING_DB_PATH")
        .or_else(|| std::env::var_os("ARCHON_EVIDENCE_DB_PATH"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| working_dir.join(".archon").join("archon-data.db"));
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match archon_learning::cozo_guard::open_sqlite_guarded(
        db_path.to_string_lossy().as_ref(),
        "open governed learning db",
    ) {
        Ok(db) => {
            if let Err(e) = archon_learning::schema::ensure_learning_schema(&db) {
                tracing::warn!(
                    error = %e,
                    "governed learning schema init failed; runtime evidence disabled"
                );
                None
            } else {
                Some(Arc::new(db))
            }
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "governed learning store unavailable; runtime evidence disabled"
            );
            None
        }
    }
}

fn open_cognitive_store(
    working_dir: &std::path::Path,
) -> Option<archon_cognitive::PersistentCognitiveStore> {
    let root = working_dir.join(".archon").join("cognitive");
    match archon_cognitive::PersistentCognitiveStore::open(&root) {
        Ok(store) => {
            tracing::info!(
                path = %store.db_path().display(),
                "cognitive executive store wired"
            );
            Some(store)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %root.display(),
                "cognitive executive store unavailable; continuing without persistence"
            );
            None
        }
    }
}

fn configure_session_vlm_provider(working_dir: &std::path::Path) {
    match archon_policy::load_effective_policy(working_dir) {
        Ok(policy) => {
            let report = archon_docs::vlm::factory::configure_registered_provider(&policy);
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
        crate::runtime::provider_auth_selection::selected_provider_auth_profile_id(provider.name());
    Ok(observe_llm_provider_with_profile(
        provider,
        runtime_mode,
        profile_id,
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_session_model_uses_configured_codex_default_when_claude_default_would_leak() {
        let mut config = archon_core::config::ArchonConfig::default();
        config.llm.provider = "openai-codex".into();
        config.api.default_model = "claude-sonnet-4-6".into();
        config.models.openai_codex.default = "gpt-codex-default".into();

        assert_eq!(active_session_model(&config), "gpt-codex-default");
    }

    #[test]
    fn active_session_model_uses_configured_codex_mini_for_haiku_default() {
        let mut config = archon_core::config::ArchonConfig::default();
        config.llm.provider = "openai-codex".into();
        config.api.default_model = "claude-haiku-4-5-20251001".into();
        config.models.openai_codex.mini = "gpt-codex-mini".into();

        assert_eq!(active_session_model(&config), "gpt-codex-mini");
    }

    #[test]
    fn active_session_model_preserves_explicit_codex_model_override() {
        let mut config = archon_core::config::ArchonConfig::default();
        config.llm.provider = "openai-codex".into();
        config.api.default_model = "gpt-5.4-codex-test".into();

        assert_eq!(active_session_model(&config), "gpt-5.4-codex-test");
    }

    #[test]
    fn active_session_model_preserves_anthropic_default() {
        let config = archon_core::config::ArchonConfig::default();

        assert_eq!(active_session_model(&config), config.api.default_model);
    }
}

pub(crate) async fn run_interactive_session(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    env_vars: &ArchonEnvVars,
    resume_messages: Option<Vec<serde_json::Value>>,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    voice_event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<archon_tui::app::TuiEvent>>,
) -> Result<()> {
    let interactive_bootstrap::Bootstrap {
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
    } = interactive_agent::build(
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
        show_thinking,
        session_stats_shared,
        last_assistant_response_shared,
    } = interactive_finish::finish(
        &mut agent,
        config,
        session_id,
        cli,
        config_path.clone(),
        working_dir.clone(),
        Arc::clone(&memory),
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
        btw_system_prompt,
        active_model,
        auto_capture,
    )
    .await
}
