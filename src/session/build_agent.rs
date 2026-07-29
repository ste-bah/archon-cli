use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::build_prompt::build_system_prompt;
use crate::cli_args::Cli;
use crate::command::utils::apply_tool_filters;
use archon_core::agent::{Agent, AgentConfig, TimestampedEvent};
use archon_core::agents::AgentRegistry;
use archon_core::agents::permissions_overlay::{
    PermissionOverlayReason, resolve_permission_overlay,
};
use archon_core::dispatch::create_default_registry;
use archon_core::env_vars::ArchonEnvVars;
use archon_llm::effort::EffortLevel;
use archon_observability::ChannelMetricSink;

#[path = "build_agent_catalog.rs"]
mod agent_catalog;
#[path = "build_agent_provider.rs"]
mod provider;
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
    let initial_effort = cli
        .effort
        .as_deref()
        .and_then(|value| archon_llm::effort::parse_level(value).ok())
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
    if let Err(e) = super::spawn_metrics_exporter(cli.metrics_port, Arc::clone(&metrics)) {
        let audit_result = super::drain_startup_sandbox_audit(sandbox_audit_drain).await;
        let error = super::finish_startup_failure(e, audit_result);
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
    let metrics_sink: Arc<dyn ChannelMetricSink> = metrics.clone();
    agent.set_channel_metrics(metrics_sink);
    if let Some(store) = cognitive_store {
        agent.set_cognitive_store(store);
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

pub(super) fn register_agent_listing(
    registry: &mut archon_core::dispatch::ToolRegistry,
    agent_registry: &AgentRegistry,
) {
    let agents: Vec<(String, String)> = agent_registry
        .list()
        .iter()
        .map(|a| (a.agent_type.clone(), a.description.clone()))
        .collect();
    let common_agents = agent_catalog::common_inline_agents(&agents);
    registry.register(Box::new(
        archon_tools::agent_tool::AgentTool::with_agent_listing(&common_agents),
    ));
    registry.register(Box::new(archon_tools::agent_tool::AgentCatalogTool::new(
        agents,
    )));
}

pub(super) async fn resolve_agent_definition(
    config: &archon_core::config::ArchonConfig,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    agent_registry: &AgentRegistry,
) -> Result<Option<archon_core::agents::definition::CustomAgentDefinition>, i32> {
    let Some(agent_name) = resolved_flags.agent.as_ref() else {
        return Ok(None);
    };
    match agent_registry.resolve(agent_name) {
        Some(def) => {
            tracing::info!(agent = agent_name, "resolved custom agent");
            let mut def = def.clone();
            if let Err(error) = crate::runtime::agent_profile_overlay::apply_active_profile_overlay_if_enabled_async(
                config, &mut def,
            )
            .await
            {
                tracing::warn!(agent = agent_name, %error, "agent profile overlay skipped");
            }
            Ok(Some(def))
        }
        None => {
            eprintln!(
                "Unknown agent '{}'. Available: {}",
                agent_name,
                agent_registry.available_agent_names().join(", ")
            );
            Err(1)
        }
    }
}

pub(super) fn apply_agent_tool_filters(
    registry: &mut archon_core::dispatch::ToolRegistry,
    agent_def: Option<&archon_core::agents::definition::CustomAgentDefinition>,
) {
    if let Some(def) = agent_def {
        if let Some(ref allowed) = def.allowed_tools {
            let allowed_refs: Vec<&str> = allowed.iter().map(|s| s.as_str()).collect();
            registry.filter_whitelist(&allowed_refs);
        }
        if let Some(ref denied) = def.disallowed_tools {
            let denied_refs: Vec<&str> = denied.iter().map(|s| s.as_str()).collect();
            registry.filter_blacklist(&denied_refs);
        }
    }
}

pub(super) fn validate_required_mcp_servers(
    registry: &archon_core::dispatch::ToolRegistry,
    agent_def: Option<&archon_core::agents::definition::CustomAgentDefinition>,
) -> Result<(), i32> {
    if let Some(def) = agent_def {
        let available_tools = registry.tool_names();
        let available_mcp: Vec<String> = available_tools
            .iter()
            .filter(|n| n.starts_with("mcp__"))
            .map(|n| n.to_string())
            .collect();
        if !def.has_required_mcp_servers(&available_mcp) {
            eprintln!(
                "Agent '{}' requires MCP servers {:?} but they are not available.",
                def.agent_type, def.required_mcp_servers,
            );
            return Err(1);
        }
    }
    Ok(())
}

pub(super) async fn apply_agent_execution_overrides(
    agent_config: &mut AgentConfig,
    agent_def: Option<&archon_core::agents::definition::CustomAgentDefinition>,
    cli: &Cli,
) {
    let Some(def) = agent_def else {
        return;
    };
    if let Some(ref model) = def.model
        && model != "inherit"
    {
        agent_config.model = model.clone();
        *agent_config.model_override.lock().await = model.clone();
    }
    if let Some(ref effort) = def.effort {
        if let Ok(level) = effort.parse::<archon_llm::effort::EffortLevel>() {
            *agent_config.effort_level.lock().await = level;
        } else {
            tracing::warn!(agent = %def.agent_type, effort = %effort, "invalid effort level in agent definition, using default");
        }
    }
    if let Some(ref pm) = def.permission_mode {
        let parent_mode = agent_config.permission_mode.lock().await.clone();
        let decision =
            resolve_permission_overlay(&parent_mode, Some(pm), cli.dangerously_skip_permissions);
        match decision.reason {
            PermissionOverlayReason::Applied => {
                *agent_config.permission_mode.lock().await =
                    decision.effective_mode.as_str().to_string();
            }
            PermissionOverlayReason::ParentModeLocked => {
                tracing::debug!(agent = %def.agent_type, parent_mode = %decision.parent_mode, requested_mode = %decision.requested_mode.expect("requested mode exists"), "agent permission_mode skipped because parent mode has priority");
            }
            PermissionOverlayReason::BlockedDangerousBypass => {
                tracing::warn!(agent = %def.agent_type, raw_mode = %pm, "agent requests bypassPermissions but --dangerously-skip-permissions not passed; ignoring");
            }
            PermissionOverlayReason::BlockedExpansion => {
                tracing::warn!(agent = %def.agent_type, parent_mode = %decision.parent_mode, requested_mode = %decision.requested_mode.expect("requested mode exists"), "agent permission_mode would widen parent mode; keeping parent mode");
            }
            PermissionOverlayReason::NoRequest => {}
        }
    }
    if def.max_turns.is_some() {
        agent_config.max_turns = def.max_turns;
    }
}
