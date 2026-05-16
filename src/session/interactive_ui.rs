use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::cli_args::Cli;
use anyhow::Result;
use archon_core::agent::{Agent, SessionStats};
use archon_core::env_vars::ArchonEnvVars;
use archon_llm::auth::resolve_auth_with_keys;
use archon_llm::effort::{EffortLevel, EffortState};
use archon_llm::fast_mode::FastModeState;
use archon_memory::MemoryTrait;
use archon_tui::app::TuiEvent;
use archon_tui::commands::CommandInfo;
use archon_tui::event_channel::{TuiEventReceiver, TuiEventSender};
use archon_tui::observability;

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    env_vars: &ArchonEnvVars,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    config_path: PathBuf,
    layer_filter: Option<Vec<archon_core::config_layers::ConfigLayer>>,
    working_dir: PathBuf,
    session_store: Arc<archon_session::storage::SessionStore>,
    memory: Arc<dyn MemoryTrait>,
    mut agent: Agent,
    agent_def: Option<archon_core::agents::definition::CustomAgentDefinition>,
    session_api_url: Option<String>,
    provider: Arc<dyn archon_llm::provider::LlmProvider>,
    mcp_manager: archon_mcp::lifecycle::McpServerManager,
    cron_shutdown: archon_tools::cron_shutdown::CronShutdown,
    fast_mode_shared: Arc<AtomicBool>,
    fast_mode: FastModeState,
    effort_level_shared: Arc<tokio::sync::Mutex<EffortLevel>>,
    effort_state: EffortState,
    model_override_shared: Arc<tokio::sync::Mutex<String>>,
    permission_mode_shared: Arc<tokio::sync::Mutex<String>>,
    extra_dirs_shared: Arc<tokio::sync::Mutex<Vec<PathBuf>>>,
    show_thinking: Arc<AtomicBool>,
    session_stats_shared: Arc<tokio::sync::Mutex<SessionStats>>,
    last_assistant_response_shared: Arc<tokio::sync::Mutex<String>>,
    system_prompt_chars: usize,
    tool_defs_chars: usize,
    agent_registry_for_skills: Arc<std::sync::RwLock<archon_core::agents::AgentRegistry>>,
    task_service: Arc<dyn archon_core::tasks::TaskService>,
    coding_pipeline: Arc<archon_pipeline::coding::facade::CodingFacade>,
    research_pipeline: Arc<archon_pipeline::research::facade::ResearchFacade>,
    llm_adapter: Arc<dyn archon_pipeline::runner::LlmClient>,
    leann: Option<Arc<archon_pipeline::runner::LeannIntegration>>,
    sandbox_flag: Arc<AtomicBool>,
    hook_registry: Arc<archon_core::hooks::HookRegistry>,
    learning_cozo_db: Option<Arc<cozo::DbInstance>>,
    governed_learning_db: Option<Arc<cozo::DbInstance>>,
    auto_trainer: Option<Arc<archon_pipeline::learning::gnn::auto_trainer::AutoTrainer>>,
    leann_init_cancel: Arc<AtomicBool>,
    agent_event_tx_for_dispatcher: tokio::sync::mpsc::UnboundedSender<
        archon_core::agent::TimestampedEvent,
    >,
    tui_event_tx: TuiEventSender,
    tui_event_rx: TuiEventReceiver,
    user_input_tx: tokio::sync::mpsc::Sender<String>,
    user_input_rx: tokio::sync::mpsc::Receiver<String>,
    perm_prompt_tx: tokio::sync::mpsc::Sender<bool>,
    btw_system_prompt: Vec<serde_json::Value>,
    active_model: String,
    auto_capture: Option<Arc<archon_pipeline::capture::AutoCapture>>,
) -> Result<()> {
    let auth_label = match resolve_auth_with_keys(
        env_vars.anthropic_api_key.as_deref(),
        env_vars.archon_api_key.as_deref(),
        env_vars.archon_oauth_token.as_deref(),
        std::env::var("ANTHROPIC_AUTH_TOKEN").ok().as_deref(),
    ) {
        Ok(archon_llm::auth::AuthProvider::OAuthToken(_)) => "OAuth".to_string(),
        Ok(archon_llm::auth::AuthProvider::CodexOAuthToken(_)) => "Codex OAuth".to_string(),
        Ok(archon_llm::auth::AuthProvider::ApiKey(_)) => "API key".to_string(),
        Ok(archon_llm::auth::AuthProvider::BearerToken(_)) => "Bearer token".to_string(),
        Err(_) => "none".to_string(),
    };
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

    let agent_dispatcher_shared: Arc<std::sync::Mutex<archon_tui::AgentDispatcher>> =
        Arc::new(std::sync::Mutex::new(archon_tui::AgentDispatcher::new(
            Arc::new(crate::agent_handle::NoopAgentRouter),
            agent_event_tx_for_dispatcher,
        )));
    let cancel_handle_slot: Arc<std::sync::Mutex<Option<Arc<crate::agent_handle::AgentHandle>>>> =
        Arc::new(std::sync::Mutex::new(None));

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
            cost_config: config.cost.clone(),
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
            last_assistant_response: last_assistant_response_shared,
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
            cancel_handle: Arc::clone(&cancel_handle_slot),
            agent_dispatcher: Arc::clone(&agent_dispatcher_shared),
            cozo_db: learning_cozo_db,
            governed_learning_db,
            auto_trainer: auto_trainer.clone(),
        });

    let slash_commands_disabled = resolved_flags.disable_slash_commands;
    let session_store_for_input = Arc::clone(&session_store);
    let session_id_for_input = session_id.to_string();
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

    let mcp_lifecycle_tx = crate::session_loop::spawn_mcp_lifecycle_task(mcp_manager.clone());
    let input_tui_tx = tui_event_tx.clone();
    observability::spawn_named(
        "session-loop",
        crate::session_loop::run_session_loop(
            agent,
            config.clone(),
            agent_def,
            session_api_url,
            input_tui_tx,
            user_input_rx,
            session_store_for_input,
            session_id_for_input,
            persist_personality,
            personality_history_limit,
            session_start_instant,
            session_start_confidence,
            slash_commands_disabled,
            fast_mode,
            effort_state,
            cmd_ctx,
            mcp_lifecycle_tx,
            auto_capture,
            auto_trainer.clone(),
            agent_dispatcher_shared,
            cancel_handle_slot,
        ),
    );

    let splash_opt = super::splash::splash_config(
        resolved_flags.bare_mode,
        &active_model,
        &working_dir,
        session_id,
    );

    let (btw_tx, btw_rx) = tokio::sync::mpsc::channel::<String>(8);
    super::btw::spawn_btw_loop(
        btw_rx,
        tui_event_tx.clone(),
        Arc::clone(&provider),
        active_model.clone(),
        config.api.thinking_budget,
        btw_system_prompt,
    );

    if config.tui.vim_mode {
        let _ = tui_event_tx.send(TuiEvent::SetVimMode(true));
    }

    let command_catalog: Vec<CommandInfo> = crate::command::registry::default_registry()
        .primaries_with_descriptions()
        .into_iter()
        .map(|(name, desc)| CommandInfo {
            name: format!("/{name}"),
            description: desc.to_string(),
        })
        .collect();

    archon_tui::app::run(archon_tui::app::AppConfig {
        event_rx: tui_event_rx,
        input_tx: user_input_tx,
        splash: splash_opt,
        btw_tx: Some(btw_tx),
        permission_tx: Some(perm_prompt_tx),
        context_window: context_resolution.context_window,
        context_source: Some(context_resolution.source.label().to_string()),
        context_threshold: config.context.compact_threshold,
        command_catalog,
    })
    .await?;

    if let Some(at) = auto_trainer.as_ref() {
        at.shutdown();
    }
    leann_init_cancel.store(true, Ordering::Relaxed);

    cron_shutdown.shutdown().await;
    tracing::info!("cron scheduler shut down");

    mcp_manager.shutdown_all().await;
    tracing::info!("MCP servers shut down");

    let alive = observability::log_alive_tasks_after_cancel(std::time::Duration::from_secs(2));
    if !alive.is_empty() {
        observability::abort_alive_tasks();
        let _ = observability::log_alive_tasks_after_cancel(std::time::Duration::from_millis(250));
    }

    Ok(())
}
