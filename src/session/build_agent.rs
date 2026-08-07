use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::build_prompt::build_system_prompt;
use crate::cli_args::Cli;
use crate::command::utils::apply_tool_filters;
use archon_core::agent::{Agent, AgentConfig, TimestampedEvent};
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::create_default_registry;
use archon_core::env_vars::ArchonEnvVars;
use archon_llm::effort::EffortLevel;
use archon_observability::ChannelMetricSink;

#[path = "build_agent_catalog.rs"]
mod agent_catalog;
#[path = "build_agent_definition.rs"]
mod agent_definition;
#[path = "build_agent_board.rs"]
mod board;
#[path = "build_agent_provider.rs"]
mod provider;
pub(super) use agent_definition::{
    apply_agent_execution_overrides, apply_agent_tool_filters, register_agent_listing,
    resolve_agent_definition, validate_required_mcp_servers,
};
use provider::{resolve_identity_and_api_client, resolve_session_provider};

pub(super) async fn build_session_agent(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    env_vars: &ArchonEnvVars,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    inject_output_style: bool,
) -> Result<super::BuiltAgent, i32> {
    let working_dir = std::env::current_dir().unwrap_or_default();
    let (identity, api_client) =
        resolve_identity_and_api_client(config, session_id, cli, env_vars).await?;

    super::configure_session_vlm_provider(&working_dir);
    // Before the registry is built, so the handle is in place by the time the
    // model can call a board tool. `BoardHandle::Global` resolves at call time,
    // so ordering against `create_default_registry` is not what matters — being
    // installed before the agent runs is.
    board::install_session_board_access(config).await;
    let leann_index = super::init_leann_index(&working_dir);
    let mut registry = create_default_registry(working_dir.clone(), leann_index);
    registry.replace(Box::new(archon_tools::bash::BashTool {
        timeout_secs: config.tools.bash_timeout,
        max_output_bytes: config.tools.bash_max_output,
        safe_commands: config.permissions.safe_commands.clone(),
        risky_commands: config.permissions.risky_commands.clone(),
        dangerous_commands: config.permissions.dangerous_commands.clone(),
        provider_env: None,
    }));
    apply_tool_filters(&mut registry, resolved_flags);

    let agent_registry_early = AgentRegistry::load(&working_dir);
    register_agent_listing(&mut registry, &agent_registry_early);
    let agent_def = resolve_agent_definition(config, resolved_flags, &agent_registry_early).await?;

    let hook_registry_arc = crate::runtime::hooks::load_runtime_hook_registry(&working_dir);
    crate::runtime::hooks::register_agent_session_hooks(
        &hook_registry_arc,
        session_id,
        agent_def.as_ref(),
    );

    apply_agent_tool_filters(&mut registry, agent_def.as_ref());
    validate_required_mcp_servers(&registry, agent_def.as_ref())?;

    let system_prompt = build_system_prompt(
        config,
        resolved_flags,
        cli,
        &working_dir,
        &identity,
        agent_def.as_ref(),
        inject_output_style,
    );

    let tool_defs = registry.tool_definitions();
    let fast_mode_shared = Arc::new(AtomicBool::new(cli.fast));
    let sandbox_flag = Arc::new(AtomicBool::new(false));
    // Precedence: --effort flag, then `[api] default_effort` (which is also
    // what ARCHON_EFFORT sets), then Medium. Mirrors the interactive path in
    // `interactive_bootstrap`.
    //
    // #123: this used to read the flag ONLY, so a headless run silently ran at
    // Medium no matter what the config or ARCHON_EFFORT said — another knob
    // that reported success and did nothing.
    let initial_effort = cli
        .effort
        .as_deref()
        .and_then(|value| archon_llm::effort::parse_level(value).ok())
        .or_else(|| archon_llm::effort::parse_level(&config.api.default_effort).ok())
        .unwrap_or(EffortLevel::Medium);
    let effort_level_shared = Arc::new(tokio::sync::Mutex::new(initial_effort));
    let model_override_shared = Arc::new(tokio::sync::Mutex::new(String::new()));
    let initial_perm_mode = if cli.dangerously_skip_permissions {
        "bypassPermissions".to_string()
    } else if let Some(ref pm) = cli.permission_mode {
        pm.clone()
    } else {
        config.permissions.mode.clone()
    };
    let permission_mode_shared = Arc::new(tokio::sync::Mutex::new(initial_perm_mode));
    let sandbox_backend = super::native_session_sandbox_backend(config, sandbox_flag).await;

    let mut agent_config = AgentConfig {
        model: super::active_session_model(config),
        max_tokens: config.api.thinking_budget,
        thinking_budget: config.api.thinking_budget,
        system_prompt,
        tools: tool_defs,
        working_dir: working_dir.clone(),
        session_id: session_id.to_string(),
        agent_type: agent_def
            .as_ref()
            .map(|def| def.agent_type.clone())
            .unwrap_or_else(|| "main".into()),
        agent_version: agent_def.as_ref().map(|def| def.meta.version.clone()),
        fast_mode: fast_mode_shared,
        effort_level: effort_level_shared,
        model_override: model_override_shared,
        permission_mode: permission_mode_shared,
        permission_rules: archon_permissions::rules::RuleSet {
            always_allow: config.permissions.always_allow.clone(),
            always_deny: config.permissions.always_deny.clone(),
            always_ask: config.permissions.always_ask.clone(),
        },
        extra_dirs: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        max_tool_concurrency: config.tools.max_concurrency as usize,
        max_turns: None,
        cancel_token: None,
        sandbox: Some(sandbox_backend),
        activity_sink: super::session_activity_sink(session_id),
        context: config.context.clone(),
        max_subagent_concurrency: config.subagent.max_concurrent,
    };
    apply_agent_execution_overrides(&mut agent_config, agent_def.as_ref(), cli).await;

    let (agent_event_tx, agent_event_rx) = tokio::sync::mpsc::channel::<TimestampedEvent>(
        archon_core::agent::AGENT_EVENT_CHANNEL_CAPACITY,
    );
    let selected_model = agent_config.model.clone();
    let permission_mode_for_built = Arc::clone(&agent_config.permission_mode);

    let provider = resolve_session_provider(
        config,
        session_id,
        &working_dir,
        &hook_registry_arc,
        api_client,
    )
    .await?;
    let selected_provider = provider.name().to_string();

    let agent_registry = Arc::new(std::sync::RwLock::new(AgentRegistry::load(&working_dir)));
    {
        let reg = agent_registry.read().expect("agent registry lock");
        tracing::info!(count = reg.len(), "loaded agent definitions");
        for err in reg.load_errors() {
            tracing::warn!(%err, "agent load error");
        }
    }

    let native_sandbox = agent_config
        .sandbox
        .take()
        .expect("session sandbox configured");
    let (sandbox, sandbox_audit_drain) = crate::runtime::sandbox_audit::audit_sandbox_backend(
        native_sandbox,
        config,
        session_id,
        &agent_config.agent_type,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "sandbox audit startup failed");
        archon_core::print_mode::EXIT_ERROR
    })?;
    agent_config.sandbox = Some(sandbox);
    let cognitive_store = match super::open_cognitive_store(&working_dir).await {
        Ok(store) => store,
        Err(error) => {
            let audit_result = super::drain_startup_sandbox_audit(sandbox_audit_drain).await;
            let error = super::finish_startup_failure(error, audit_result);
            tracing::error!(%error, "cognitive executive store startup failed");
            return Err(archon_core::print_mode::EXIT_ERROR);
        }
    };
    let metrics = Arc::new(archon_tui::observability::ChannelMetrics::default());
    if let Err(error) = super::spawn_metrics_exporter(cli.metrics_port, Arc::clone(&metrics)) {
        let audit_result = super::drain_startup_sandbox_audit(sandbox_audit_drain).await;
        let error = super::finish_startup_failure(error, audit_result);
        eprintln!("Metrics exporter failed: {error}");
        return Err(archon_core::print_mode::EXIT_ERROR);
    }

    let mut agent = Agent::new(
        provider,
        registry,
        agent_config,
        agent_event_tx,
        agent_registry,
    );
    super::world_model_callbacks::install(&mut agent, config, session_id);
    let session_store =
        open_noninteractive_session_store(config, session_id, &working_dir, &selected_model)?;
    agent.set_session_store(session_store);
    let metrics_sink: Arc<dyn ChannelMetricSink> = metrics.clone();
    agent.set_channel_metrics(metrics_sink);
    if let Some(store) = cognitive_store {
        super::cognitive_store::wire_runtime(&mut agent, config, &working_dir, store);
    }

    agent.set_hook_registry(Arc::clone(&hook_registry_arc));
    agent.set_auto_evaluator(archon_permissions::auto::AutoModeEvaluator::new(
        archon_permissions::auto::AutoModeConfig {
            safe_commands: config.permissions.safe_commands.clone(),
            risky_commands: config.permissions.risky_commands.clone(),
            dangerous_commands: config.permissions.dangerous_commands.clone(),
            allow_paths: config.permissions.allow_paths.clone(),
            deny_paths: config.permissions.deny_paths.clone(),
            project_dir: Some(working_dir),
        },
    ));
    agent.install_subagent_executor();

    if let Some(ref def) = agent_def
        && let Some(ref reminder) = def.critical_system_reminder
    {
        agent.set_critical_system_reminder(reminder.clone());
    }

    Ok(super::BuiltAgent {
        agent,
        event_rx: agent_event_rx,
        agent_def,
        metrics,
        selected_provider,
        selected_model,
        permission_mode: permission_mode_for_built,
        sandbox_audit_drain,
    })
}

fn open_noninteractive_session_store(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    working_dir: &std::path::Path,
    model: &str,
) -> Result<Arc<archon_session::storage::SessionStore>, i32> {
    let path = crate::command::store_paths::session_db_path(config);
    let store = archon_session::storage::SessionStore::open(&path).map_err(|error| {
        tracing::error!(%error, path = %path.display(), "failed to open session store");
        archon_core::print_mode::EXIT_ERROR
    })?;
    if store.get_session(session_id).is_err() {
        store
            .register_session(session_id, &working_dir.display().to_string(), None, model)
            .map_err(|error| {
                tracing::error!(%error, "failed to register non-interactive session");
                archon_core::print_mode::EXIT_ERROR
            })?;
    }
    Ok(Arc::new(store))
}
