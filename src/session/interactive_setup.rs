use std::path::PathBuf;
use std::sync::Arc;

use crate::cli_args::Cli;
use crate::command::utils::apply_tool_filters;
use anyhow::Result;
use archon_core::agent::AgentConfig;
use archon_core::agents::AgentRegistry;
use archon_memory::MemoryTrait;

pub(super) struct Setup {
    pub registry: archon_core::dispatch::ToolRegistry,
    pub agent_def: Option<archon_core::agents::definition::CustomAgentDefinition>,
    pub active_model: String,
    pub permission_mode_shared: Arc<tokio::sync::Mutex<String>>,
    pub btw_system_prompt: Vec<serde_json::Value>,
    pub system_prompt_chars: usize,
    pub tool_defs_chars: usize,
    pub agent_config: AgentConfig,
    pub cron_shutdown: archon_tools::cron_shutdown::CronShutdown,
}

pub(super) async fn prepare(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    session_store: Arc<archon_session::storage::SessionStore>,
    memory: Arc<dyn MemoryTrait>,
    working_dir: PathBuf,
    prompt_identity: archon_llm::identity::IdentityProvider,
    mcp_tools: Vec<archon_mcp::tool_bridge::McpTool>,
    fast_mode_shared: Arc<std::sync::atomic::AtomicBool>,
    effort_level_shared: Arc<tokio::sync::Mutex<archon_llm::effort::EffortLevel>>,
    model_override_shared: Arc<tokio::sync::Mutex<String>>,
    sandbox_flag: Arc<std::sync::atomic::AtomicBool>,
) -> Result<Setup> {
    let leann_index = super::init_leann_index(&working_dir);
    let mut registry = archon_core::dispatch::create_default_registry(working_dir.clone(), leann_index);
    registry.register(Box::new(archon_tools::bash::BashTool {
        timeout_secs: config.tools.bash_timeout,
        max_output_bytes: config.tools.bash_max_output,
    }));
    apply_tool_filters(&mut registry, resolved_flags);

    {
        use archon_plugin::{api::tools_from_plugin_instance, loader::instantiate_wasm_plugins};
        let plugins_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("archon")
            .join("plugins");
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from(".cache"))
            .join("archon")
            .join("wasm");
        let plugin_result = archon_plugin::loader::PluginLoader::new(plugins_dir)
            .with_cache(archon_plugin::cache::WasmCache::new(cache_dir))
            .load_all();
        let wasm_instances = instantiate_wasm_plugins(&plugin_result);
        for (plugin_id, (instance, host)) in wasm_instances {
            let tools = tools_from_plugin_instance(&plugin_id, &instance, host);
            let count = tools.len();
            for tool in tools {
                registry.register(tool);
            }
            if count > 0 {
                tracing::info!(plugin = %plugin_id, count, "registered WASM plugin tools");
            }
        }
    }

    let cron_shutdown = {
        let cron_store_path = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("archon")
            .join("scheduled_tasks.json");
        archon_tools::cron_shutdown::spawn_scheduler(cron_store_path)
    };

    if config.memory.enabled {
        registry.register(Box::new(archon_tools::memory::MemoryStoreTool::new(
            Arc::clone(&memory),
        )));
        registry.register(Box::new(archon_tools::memory::MemoryRecallTool::new(
            Arc::clone(&memory),
        )));
    }

    let verbosity_state = Arc::new(std::sync::Mutex::new(
        archon_tools::verbosity_toggle::VerbosityState::new(config.tui.verbose),
    ));
    registry.register(Box::new(
        archon_tools::verbosity_toggle::VerbosityToggleTool::new(Arc::clone(&verbosity_state)),
    ));

    let mcp_tool_count = mcp_tools.len();
    for tool in mcp_tools {
        registry.register(Box::new(tool));
    }
    if mcp_tool_count > 0 {
        tracing::info!("registered {mcp_tool_count} MCP tools into agent registry");
    }

    let agent_registry_tmp = AgentRegistry::load(&working_dir);
    super::build_agent::register_agent_listing(&mut registry, &agent_registry_tmp);
    let agent_def =
        super::build_agent::resolve_agent_definition(config, resolved_flags, &agent_registry_tmp)
            .map_err(|code| anyhow::anyhow!("agent resolution failed with exit code {code}"))?;
    drop(agent_registry_tmp);

    super::build_agent::apply_agent_tool_filters(&mut registry, agent_def.as_ref());
    super::build_agent::validate_required_mcp_servers(&registry, agent_def.as_ref())
        .map_err(|code| anyhow::anyhow!("required MCP servers unavailable (exit code {code})"))?;

    let active_model = super::active_session_model(config);
    let git_info = archon_core::git::detect_git_info(&working_dir);
    let git_branch = git_info.as_ref().map(|g| g.branch.as_str());
    if let Err(e) = session_store.register_session(
        session_id,
        &working_dir.display().to_string(),
        git_branch,
        &active_model,
    ) {
        tracing::warn!("failed to register session: {e}");
    }
    if let Some(ref name) = cli.session_name {
        if let Err(e) = archon_session::naming::set_session_name(&session_store, session_id, name) {
            tracing::warn!("failed to set session name: {e}");
        } else {
            tracing::info!("session named: {name}");
        }
    }

    let system_prompt = super::build_prompt::build_interactive_system_prompt(
        config,
        resolved_flags,
        cli,
        &working_dir,
        session_id,
        &prompt_identity,
        agent_def.as_ref(),
        memory.as_ref(),
    );
    let tool_defs = registry.tool_definitions();

    let initial_perm_mode = if cli.dangerously_skip_permissions {
        "bypassPermissions".to_string()
    } else if let Some(ref pm) = cli.permission_mode {
        pm.clone()
    } else {
        config.permissions.mode.clone()
    };
    let permission_mode_shared = Arc::new(tokio::sync::Mutex::new(initial_perm_mode));

    let btw_system_prompt = system_prompt.clone();
    let system_prompt_chars: usize = system_prompt
        .iter()
        .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
        .map(|s| s.len())
        .sum();
    let tool_defs_chars: usize = tool_defs
        .iter()
        .map(|t| serde_json::to_string(t).unwrap_or_default().len())
        .sum();

    let mut agent_config = AgentConfig {
        model: active_model.clone(),
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
        fast_mode: Arc::clone(&fast_mode_shared),
        effort_level: Arc::clone(&effort_level_shared),
        model_override: Arc::clone(&model_override_shared),
        permission_mode: Arc::clone(&permission_mode_shared),
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
    super::build_agent::apply_agent_execution_overrides(&mut agent_config, agent_def.as_ref(), cli)
        .await;

    Ok(Setup {
        registry,
        agent_def,
        active_model,
        permission_mode_shared,
        btw_system_prompt,
        system_prompt_chars,
        tool_defs_chars,
        agent_config,
        cron_shutdown,
    })
}
