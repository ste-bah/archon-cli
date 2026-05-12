use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::build_prompt::build_system_prompt;
use crate::cli_args::Cli;
use crate::command::utils::{apply_tool_filters, fetch_account_uuid};
use crate::runtime::llm::{
    build_llm_provider_selection, provider_construction_error_reason,
    record_anthropic_fallback_denied,
};
use crate::runtime::llm_non_anthropic::build_llm_provider_without_anthropic_fallback;
use crate::runtime::provider_observer::{
    observe_llm_provider_with_profile, record_provider_fallback, runtime_mode_for_provider_name,
};
use archon_core::agent::{Agent, AgentConfig, TimestampedEvent};
use archon_core::agents::AgentRegistry;
use archon_core::agents::permissions_overlay::{
    PermissionOverlayReason, resolve_permission_overlay,
};
use archon_core::dispatch::create_default_registry;
use archon_core::env_vars::ArchonEnvVars;
use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::resolve_auth_with_keys;
use archon_llm::effort::EffortLevel;
use archon_llm::identity::{
    IdentityMode, IdentityProvider, get_or_create_device_id, resolve_identity_mode,
};
use archon_observability::ChannelMetricSink;

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
    registry.register(Box::new(archon_tools::bash::BashTool {
        timeout_secs: config.tools.bash_timeout,
        max_output_bytes: config.tools.bash_max_output,
    }));
    apply_tool_filters(&mut registry, resolved_flags);

    let agent_registry_early = AgentRegistry::load(&working_dir);
    register_agent_listing(&mut registry, &agent_registry_early);
    let agent_def = resolve_agent_definition(config, resolved_flags, &agent_registry_early)?;

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
        sandbox: Some(super::session_sandbox_backend(
            config,
            sandbox_flag,
            session_id,
            agent_def
                .as_ref()
                .map(|def| def.agent_type.as_str())
                .unwrap_or("main"),
        )),
        activity_sink: super::session_activity_sink(session_id),
    };
    apply_agent_execution_overrides(&mut agent_config, agent_def.as_ref(), cli).await;

    let (agent_event_tx, agent_event_rx) =
        tokio::sync::mpsc::unbounded_channel::<TimestampedEvent>();
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

    let mut agent = Agent::new(
        provider,
        registry,
        agent_config,
        agent_event_tx,
        agent_registry,
    );
    let metrics = Arc::new(archon_tui::observability::ChannelMetrics::default());
    let metrics_sink: Arc<dyn ChannelMetricSink> = metrics.clone();
    agent.set_channel_metrics(metrics_sink);

    if let Err(e) = super::spawn_metrics_exporter(cli.metrics_port, Arc::clone(&metrics)) {
        eprintln!("Metrics exporter failed: {e}");
        return Err(archon_core::print_mode::EXIT_ERROR);
    }

    agent.set_hook_registry(Arc::clone(&hook_registry_arc));
    agent.set_auto_evaluator(archon_permissions::auto::AutoModeEvaluator::new(
        archon_permissions::auto::AutoModeConfig {
            project_dir: Some(working_dir),
            ..Default::default()
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
    })
}

async fn resolve_identity_and_api_client(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    env_vars: &ArchonEnvVars,
) -> Result<(IdentityProvider, Option<AnthropicClient>), i32> {
    let device_id = get_or_create_device_id();
    if super::is_codex_session(config) || config.llm.provider != "anthropic" {
        return Ok((
            IdentityProvider::new(
                IdentityMode::Clean,
                session_id.to_string(),
                device_id,
                String::new(),
            ),
            None,
        ));
    }

    let auth = match resolve_auth_with_keys(
        env_vars.anthropic_api_key.as_deref(),
        env_vars.archon_api_key.as_deref(),
        env_vars.archon_oauth_token.as_deref(),
        std::env::var("ANTHROPIC_AUTH_TOKEN").ok().as_deref(),
    ) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Authentication failed: {e}");
            eprintln!("Run `archon login` or set ANTHROPIC_API_KEY.");
            return Err(archon_core::print_mode::EXIT_ERROR);
        }
    };
    let identity_mode =
        resolve_identity_mode(&auth, cli.identity_spoof, &config.identity.as_view());
    let account_uuid = fetch_account_uuid(&auth).await;
    let identity = IdentityProvider::new(
        identity_mode,
        session_id.to_string(),
        device_id,
        account_uuid,
    );
    let api_url = std::env::var("ANTHROPIC_BASE_URL")
        .ok()
        .or_else(|| config.api.base_url.clone());
    Ok((
        identity.clone(),
        Some(AnthropicClient::new(auth, identity, api_url)),
    ))
}

async fn resolve_session_provider(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    working_dir: &std::path::Path,
    hook_registry: &Arc<archon_core::hooks::HookRegistry>,
    api_client: Option<AnthropicClient>,
) -> Result<Arc<dyn archon_llm::provider::LlmProvider>, i32> {
    let requested_provider = if super::is_codex_session(config) {
        "openai-codex"
    } else {
        config.llm.provider.as_str()
    };
    crate::runtime::hooks::fire_provider_resolve_hook(
        hook_registry,
        working_dir,
        session_id,
        crate::runtime::hooks::ProviderResolveHookPayload {
            hook_event: "BeforeProviderResolve",
            stage: "before_provider_resolve",
            surface: "session_agent",
            requested_provider,
            selected_provider: None,
            runtime_mode: None,
            profile_id: None,
        },
    )
    .await;

    let provider = if super::is_codex_session(config) {
        let (provider, runtime_mode) =
            match crate::runtime::codex_provider::build_codex_provider(config, "session_agent")
                .await
            {
                Ok(provider) => provider,
                Err(error) => {
                    eprintln!("Codex provider failed: {error}");
                    return Err(archon_core::print_mode::EXIT_ERROR);
                }
            };
        let profile_id = crate::runtime::provider_auth_selection::selected_provider_auth_profile_id(
            provider.name(),
        );
        observe_llm_provider_with_profile(provider, runtime_mode, profile_id)
    } else {
        let provider = match api_client {
            Some(api_client) => {
                let selection =
                    build_llm_provider_selection(&config.llm, &config.models, api_client);
                let selected_provider = selection.provider.name().to_string();
                let runtime_mode = runtime_mode_for_provider_name(&selected_provider);
                record_provider_fallback(
                    &config.llm.provider,
                    &selected_provider,
                    runtime_mode,
                    selection
                        .fallback_reason
                        .unwrap_or("provider_construction_fallback"),
                );
                selection.provider
            }
            None => match build_llm_provider_without_anthropic_fallback(&config.llm) {
                Ok(provider) => provider,
                Err(error) => {
                    let reason = provider_construction_error_reason(&error);
                    record_anthropic_fallback_denied(&config.llm.provider, "session_agent", reason);
                    eprintln!("Provider {} failed: {error}", config.llm.provider);
                    return Err(archon_core::print_mode::EXIT_ERROR);
                }
            },
        };
        let selected_provider = provider.name().to_string();
        let runtime_mode = runtime_mode_for_provider_name(&selected_provider);
        let profile_id = crate::runtime::provider_auth_selection::selected_provider_auth_profile_id(
            &selected_provider,
        );
        observe_llm_provider_with_profile(provider, runtime_mode, profile_id)
    };

    let selected_provider = provider.name().to_string();
    crate::runtime::hooks::fire_provider_resolve_hook(
        hook_registry,
        working_dir,
        session_id,
        crate::runtime::hooks::ProviderResolveHookPayload {
            hook_event: "AfterProviderResolve",
            stage: "after_provider_resolve",
            surface: "session_agent",
            requested_provider,
            selected_provider: Some(&selected_provider),
            runtime_mode: Some(runtime_mode_for_provider_name(&selected_provider)),
            profile_id: crate::runtime::provider_auth_selection::selected_provider_auth_profile_id(
                &selected_provider,
            )
            .as_deref(),
        },
    )
    .await;
    tracing::info!("LLM provider: {}", provider.name());
    Ok(provider)
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
    registry.register(Box::new(
        archon_tools::agent_tool::AgentTool::with_agent_listing(&agents),
    ));
}

pub(super) fn resolve_agent_definition(
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
            if let Err(error) =
                crate::runtime::agent_profile_overlay::apply_active_profile_overlay_if_enabled(
                    config, &mut def,
                )
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
