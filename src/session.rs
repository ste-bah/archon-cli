//! Print-mode session runner. Extracted from main.rs.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::cli_args::Cli;
use crate::slash_context::SlashCommandContext;
use anyhow::Result;
use archon_consciousness::assembler::{AssemblyInput, BudgetConfig, SystemPromptAssembler};
use archon_consciousness::defaults::load_configured_defaults;
use archon_consciousness::rules::RulesEngine;
use archon_core::agent::{Agent, AgentConfig, AgentEvent, TimestampedEvent};
use archon_core::agents::AgentRegistry;
use archon_core::config::default_config_path;
use archon_core::config_layers::ConfigLayer;
use archon_core::cost_alerts::{CostAlertAction, CostAlertState};
use archon_core::dispatch::create_default_registry;
use archon_core::env_vars::ArchonEnvVars;
use archon_core::print_mode::{PrintModeConfig, run_print_mode};
use archon_core::reasoning::build_environment_section;
use archon_core::skills::builtin::register_builtins;
use archon_core::skills::discovery::discover_user_skills;
use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::resolve_auth_with_keys;
use archon_llm::effort::{self, EffortLevel, EffortState};
use archon_llm::fast_mode::FastModeState;
use archon_llm::identity::{
    IdentityMode, IdentityProvider, get_or_create_device_id, resolve_and_validate_betas,
    resolve_betas,
};
use archon_memory::{MemoryAccess, MemoryGraph, MemoryTrait};
use archon_permissions::auto::{AutoModeConfig, AutoModeEvaluator};
use archon_tui::app::TuiEvent;
use archon_tui::observability;

use crate::runtime::llm::build_llm_provider;
use crate::setup::strip_cache_control_if_disabled;

/// Spawn the Prometheus `/metrics` exporter when `--metrics-port PORT` is
/// both present and non-zero. Port 0 is treated as "disabled" per the
/// documented CLI contract (otherwise `--metrics-port 0` would bind to an
/// OS-chosen ephemeral port, which is useless for scraping).
///
/// Bind failures are validated synchronously: we call `TcpListener::bind`
/// *before* spawning the serve task so a "permission denied" / "address in
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
    tokio::spawn(async move {
        if let Err(e) = observability::serve_metrics_on(tokio_listener, metrics).await {
            tracing::warn!(%e, port, "metrics exporter terminated");
        }
    });
    tracing::info!(port, "Prometheus /metrics exporter bound on 127.0.0.1");
    Ok(())
}

/// Parse `--setting-sources` names into [`ConfigLayer`] variants, warning on
/// unrecognised values.
pub(crate) fn parse_layer_filter(sources: &[String]) -> Vec<ConfigLayer> {
    sources
        .iter()
        .filter_map(|s| match s.as_str() {
            "user" => Some(ConfigLayer::User),
            "project" => Some(ConfigLayer::Project),
            "local" => Some(ConfigLayer::Local),
            other => {
                eprintln!("warning: unknown setting source: {other}");
                None
            }
        })
        .collect()
}

/// Apply `--tools` (whitelist) and `--disallowed-tools` (blacklist) from

/// Apply `--tools` (whitelist) and `--disallowed-tools` (blacklist) from
/// resolved CLI flags to the tool registry.
pub(crate) fn apply_tool_filters(
    registry: &mut archon_core::dispatch::ToolRegistry,
    flags: &archon_core::cli_flags::ResolvedFlags,
) {
    if let Some(ref whitelist) = flags.tool_whitelist {
        let names: Vec<&str> = whitelist.iter().map(|s| s.as_str()).collect();
        registry.filter_whitelist(&names);
        tracing::info!("tool whitelist applied: {} tools retained", names.len());
    }
    if let Some(ref blacklist) = flags.tool_blacklist {
        let names: Vec<&str> = blacklist.iter().map(|s| s.as_str()).collect();
        registry.filter_blacklist(&names);
        tracing::info!("tool blacklist applied: removed {} patterns", names.len());
    }
}

/// Fetch account UUID from Anthropic OAuth profile endpoint.
pub(crate) async fn fetch_account_uuid(auth: &archon_llm::auth::AuthProvider) -> String {
    let (header_name, header_value) = auth.header();

    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let result = client
        .get("https://api.anthropic.com/api/oauth/profile")
        .header(&header_name, &header_value)
        .send()
        .await;

    match result {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.text().await
                && let Ok(json) = serde_json::from_str::<serde_json::Value>(&body)
            {
                // Profile response: { "account": { "uuid": "..." }, "organization": { ... } }
                if let Some(uuid) = json
                    .get("account")
                    .and_then(|a| a.get("uuid"))
                    .and_then(|v| v.as_str())
                {
                    tracing::info!("fetched account_uuid: {}", &uuid[..8.min(uuid.len())]);
                    return uuid.to_string();
                }
            }
            tracing::warn!("profile response missing account_uuid");
            String::new()
        }
        Ok(resp) => {
            tracing::warn!("profile fetch failed: HTTP {}", resp.status());
            String::new()
        }
        Err(e) => {
            tracing::warn!("profile fetch error: {e}");
            String::new()
        }
    }
}

/// Run a print-mode session: set up auth/agent, process one query, return exit code.
pub(crate) async fn run_print_mode_session(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    env_vars: &ArchonEnvVars,
    print_config: PrintModeConfig,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
) -> i32 {
    // Resolve authentication (same as interactive)
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
            return archon_core::print_mode::EXIT_ERROR;
        }
    };

    let device_id = get_or_create_device_id();
    let betas = resolve_betas(config.identity.spoof_betas.as_deref());
    let identity_mode = if cli.identity_spoof {
        IdentityMode::Spoof {
            version: config.identity.spoof_version.clone(),
            entrypoint: config.identity.spoof_entrypoint.clone(),
            betas,
            workload: config.identity.workload.clone(),
            anti_distillation: config.identity.anti_distillation,
        }
    } else {
        IdentityMode::Clean
    };

    let account_uuid = fetch_account_uuid(&auth).await;
    let identity = IdentityProvider::new(
        identity_mode,
        session_id.to_string(),
        device_id,
        account_uuid,
    );

    // Resolve API base URL: env var > config > hardcoded default (inside AnthropicClient::new)
    let api_url = std::env::var("ANTHROPIC_BASE_URL")
        .ok()
        .or_else(|| config.api.base_url.clone());

    let api_client = AnthropicClient::new(auth, identity.clone(), api_url);
    let working_dir = std::env::current_dir().unwrap_or_default();
    let mut registry = create_default_registry(working_dir.clone());
    // Wire config-driven bash tool limits
    registry.register(Box::new(archon_tools::bash::BashTool {
        timeout_secs: config.tools.bash_timeout,
        max_output_bytes: config.tools.bash_max_output,
    }));

    // Apply tool filtering from resolved flags (CLI-220)
    apply_tool_filters(&mut registry, resolved_flags);

    // ── Resolve --agent flag against AgentRegistry (AGT-008) ──
    // Load registry early so we can resolve before tool_defs extraction.
    let agent_registry_early = AgentRegistry::load(&working_dir);

    // ── Inject agent listing into Agent tool description (AGT-011) ──
    {
        let agents: Vec<(String, String)> = agent_registry_early
            .list()
            .iter()
            .map(|a| (a.agent_type.clone(), a.description.clone()))
            .collect();
        registry.register(Box::new(
            archon_tools::agent_tool::AgentTool::with_agent_listing(&agents),
        ));
    }

    let agent_def = if let Some(ref agent_name) = resolved_flags.agent {
        match agent_registry_early.resolve(agent_name) {
            Some(def) => {
                tracing::info!(agent = agent_name, "print mode: resolved custom agent");
                Some(def.clone())
            }
            None => {
                let available = agent_registry_early.available_agent_names().join(", ");
                eprintln!("Unknown agent '{}'. Available: {}", agent_name, available);
                return 1;
            }
        }
    } else {
        None
    };

    // Apply agent tool filtering to registry
    if let Some(ref def) = agent_def {
        if let Some(ref allowed) = def.allowed_tools {
            let allowed_refs: Vec<&str> = allowed.iter().map(|s| s.as_str()).collect();
            registry.filter_whitelist(&allowed_refs);
        }
        if let Some(ref denied) = def.disallowed_tools {
            let denied_refs: Vec<&str> = denied.iter().map(|s| s.as_str()).collect();
            registry.filter_blacklist(&denied_refs);
        }
    }

    // Pre-flight check: required MCP servers must be available for --agent mode
    if let Some(ref def) = agent_def {
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
            return 1;
        }
    }

    // Build a minimal system prompt (skip ARCHON.md in bare mode)
    // Note: omit_claude_md is subagent-spawn only (matches Claude Code behavior).
    // --agent CLI mode always gets full ARCHON.md.
    let archon_md = if resolved_flags.bare_mode {
        String::new()
    } else {
        archon_core::archonmd::load_hierarchical_archon_md_with_limit(
            &working_dir,
            config.context.archonmd_max_tokens as usize,
        )
    };
    let git_info = archon_core::git::detect_git_info(&working_dir);
    let git_branch = git_info.as_ref().map(|g| g.branch.as_str());
    let env_section = build_environment_section(&working_dir, git_branch);

    let mut identity_blocks = identity.system_prompt_blocks("", &archon_md, &env_section);
    // Gated by config.context.prompt_cache (TASK-WIRE-003) — strip cache_control
    // from identity blocks when disabled so print mode honours the flag too.
    strip_cache_control_if_disabled(&mut identity_blocks, config.context.prompt_cache);
    let mut system_prompt: Vec<serde_json::Value> = identity_blocks;

    // Inject agent system prompt (replaces default personality in print mode)
    if let Some(ref def) = agent_def {
        // Clear default identity blocks and inject agent prompt instead
        let mut agent_prompt = def.system_prompt.clone();

        // Inject tool guidance
        if !def.tool_guidance.is_empty() {
            agent_prompt = format!(
                "{agent_prompt}\n\n<tool-guidance>\n{}\n</tool-guidance>",
                def.tool_guidance
            );
        }

        // Inject skills
        if let Some(ref skills) = def.skills {
            if !skills.is_empty() {
                let skills_list = skills.join(", ");
                agent_prompt = format!(
                    "{agent_prompt}\n\n<available-skills>\nThe following skills are available to you: {skills_list}\nInvoke them by name when relevant to the task.\n</available-skills>"
                );
            }
        }

        // Inject LEANN queries and memory tags
        if !def.leann_queries.is_empty() {
            let queries = def.leann_queries.join(", ");
            agent_prompt = format!(
                "{agent_prompt}\n\n<leann-queries>\nRelevant code search queries for your task: {queries}\nUse these with the LEANN semantic search tool when exploring the codebase.\n</leann-queries>"
            );
        }
        if !def.tags.is_empty() {
            let tags = def.tags.join(", ");
            agent_prompt = format!(
                "{agent_prompt}\n\n<agent-tags>\nYour memory tags: {tags}\nUse these tags when storing or recalling memories relevant to your role.\n</agent-tags>"
            );
        }

        system_prompt = vec![serde_json::json!({
            "type": "text",
            "text": agent_prompt,
        })];
    }

    // ── Output style injection for print mode (CLI-310) ──────────
    {
        use archon_core::output_style::OutputStyleRegistry;
        use archon_core::output_style_loader::load_styles_from_dir;

        let mut reg = OutputStyleRegistry::new();
        if let Some(home) = dirs::home_dir() {
            let new_dir = home.join(".archon").join("output-styles");
            if new_dir.is_dir() {
                for style in load_styles_from_dir(&new_dir) {
                    reg.register(style);
                }
            } else {
                let old_dir = home.join(".claude").join("output-styles");
                if old_dir.is_dir() {
                    tracing::warn!(
                        "Loading from deprecated path {}. Rename to {} to suppress this warning.",
                        old_dir.display(),
                        new_dir.display()
                    );
                    for style in load_styles_from_dir(&old_dir) {
                        reg.register(style);
                    }
                }
            }
        }

        let style_name = cli
            .output_style
            .as_deref()
            .or(config.output_style.as_deref());

        let injection = if let Some(name) = style_name {
            reg.get_or_default(name).prompt.clone()
        } else {
            reg.forced_plugin_style().and_then(|s| s.prompt.clone())
        };

        if let Some(ref text) = injection {
            system_prompt.push(serde_json::json!({ "type": "text", "text": text }));
        }
    }

    let tool_defs = registry.tool_definitions();

    let fast_mode_shared = Arc::new(AtomicBool::new(cli.fast));
    let initial_effort = if let Some(ref effort_arg) = cli.effort {
        archon_llm::effort::parse_level(effort_arg).unwrap_or(EffortLevel::Medium)
    } else {
        EffortLevel::Medium
    };
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
        model: config.api.default_model.clone(),
        max_tokens: config.api.thinking_budget,
        thinking_budget: config.api.thinking_budget,
        system_prompt,
        tools: tool_defs,
        working_dir: working_dir.clone(),
        session_id: session_id.to_string(),
        fast_mode: fast_mode_shared,
        effort_level: effort_level_shared,
        model_override: model_override_shared,
        permission_mode: permission_mode_shared,
        extra_dirs: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        max_tool_concurrency: config.tools.max_concurrency as usize,
        max_turns: None,
        cancel_token: None,
    };

    // Apply agent execution config overrides (AGT-008)
    if let Some(ref def) = agent_def {
        // AC-113: model="inherit" means use parent model (skip override)
        if let Some(ref m) = def.model {
            if m != "inherit" {
                agent_config.model = m.clone();
                *agent_config.model_override.blocking_lock() = m.clone();
            }
        }
        if let Some(ref e) = def.effort {
            if let Ok(level) = e.parse::<archon_llm::effort::EffortLevel>() {
                *agent_config.effort_level.blocking_lock() = level;
            } else {
                tracing::warn!(agent = %def.agent_type, effort = %e, "invalid effort level in agent definition, using default");
            }
        }
        if let Some(ref pm) = def.permission_mode {
            let mode_str = pm.as_str();
            // AC-103: Agent permission_mode must NOT override parent BypassPermissions/AcceptEdits/Auto
            let parent_mode = agent_config.permission_mode.blocking_lock().clone();
            let parent_is_privileged = matches!(
                parent_mode.as_str(),
                "bypassPermissions" | "acceptEdits" | "auto"
            );
            if parent_is_privileged {
                tracing::debug!(
                    agent = %def.agent_type, parent_mode = %parent_mode, agent_mode = %mode_str,
                    "agent permission_mode skipped — parent has privileged mode"
                );
            } else if mode_str == "bypassPermissions" && !cli.dangerously_skip_permissions {
                tracing::warn!(
                    agent = %def.agent_type, raw_mode = %pm,
                    "agent requests bypassPermissions but --dangerously-skip-permissions not passed; ignoring"
                );
            } else {
                *agent_config.permission_mode.blocking_lock() = mode_str.to_string();
            }
        }
        if def.max_turns.is_some() {
            agent_config.max_turns = def.max_turns;
        }
    }

    let (agent_event_tx, agent_event_rx) =
        tokio::sync::mpsc::unbounded_channel::<TimestampedEvent>();
    let provider = build_llm_provider(&config.llm, api_client);
    tracing::info!("LLM provider: {}", provider.name());

    // Load custom agent registry (built-in + project + user agents)
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

    // Wire channel metrics for observability (TASK-TUI-206)
    let metrics = Arc::new(archon_tui::observability::ChannelMetrics::default());
    let metrics_for_agent = Arc::clone(&metrics);
    agent.set_channel_metrics(metrics_for_agent);

    // TASK-TUI-803: spawn Prometheus /metrics exporter on loopback when
    // `--metrics-port <PORT>` is set (non-zero). Shares the metrics Arc
    // with the agent so every scrape reflects live counter state. Bind
    // errors surface synchronously so an unusable `--metrics-port 80`
    // fails the session rather than silently lying. Print-mode returns
    // i32 (not Result), so the bind error is rendered to stderr and
    // EXIT_ERROR is returned directly instead of `?`-propagating.
    if let Err(e) = spawn_metrics_exporter(cli.metrics_port, Arc::clone(&metrics)) {
        eprintln!("Metrics exporter failed: {e}");
        return archon_core::print_mode::EXIT_ERROR;
    }

    // Wire hook system for print mode — load hooks then register agent-specific hooks
    {
        let home_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let hook_registry = archon_core::hooks::HookRegistry::load_all(&working_dir, &home_dir);
        let arc = std::sync::Arc::new(hook_registry);
        agent.set_hook_registry(Arc::clone(&arc));

        // Register agent-specific hooks as session-scoped hooks
        if let Some(ref def) = agent_def {
            if let Some(ref hooks_json) = def.hooks {
                match archon_core::agents::loader::parse_agent_hooks(hooks_json) {
                    Ok(hook_pairs) => {
                        for (event, config) in hook_pairs {
                            arc.register_session_hook(session_id, event, config);
                        }
                        tracing::info!(agent = %def.agent_type, "print mode: registered agent session-scoped hooks");
                    }
                    Err(e) => {
                        tracing::warn!(agent = %def.agent_type, error = %e, "failed to parse agent hooks")
                    }
                }
            }
        }
    }

    // Wire auto-mode evaluator
    let auto_eval = AutoModeEvaluator::new(AutoModeConfig {
        project_dir: Some(working_dir),
        ..Default::default()
    });
    agent.set_auto_evaluator(auto_eval);

    // Wire subagent executor (TASK-AGS-105) — must be AFTER all post-construction
    // setters so AgentSubagentExecutor captures hook_registry, memory, etc.
    agent.install_subagent_executor();

    // Wire Phase G: critical_system_reminder for per-turn injection in print mode
    if let Some(ref def) = agent_def {
        if let Some(ref reminder) = def.critical_system_reminder {
            agent.set_critical_system_reminder(reminder.clone());
        }
    }

    // AGT-011: Prepend initial_prompt to the query in print mode
    let mut print_config = print_config;
    if let Some(ref def) = agent_def {
        if let Some(ref prefix) = def.initial_prompt {
            print_config.query = format!("{prefix}\n\n{}", print_config.query);
        }
    }

    run_print_mode(print_config, config, &mut agent, agent_event_rx).await
}

// ── Interactive session helpers ─────────────────────────────────────────────

/// List recent sessions for `--resume` with no ID.
pub(crate) async fn handle_resume_list() -> Result<()> {
    let db_path = archon_session::storage::default_db_path();
    let store = archon_session::storage::SessionStore::open(&db_path)
        .map_err(|e| anyhow::anyhow!("failed to open session database: {e}"))?;

    let sessions = store
        .list_sessions(20)
        .map_err(|e| anyhow::anyhow!("failed to list sessions: {e}"))?;

    if sessions.is_empty() {
        eprintln!("No previous sessions found.");
    } else {
        eprintln!("Recent sessions:");
        for session in &sessions {
            eprintln!("  {}", archon_session::resume::format_session_line(session));
        }
        eprintln!("\nUse: archon --resume <session-id>");
    }
    Ok(())
}

/// Load resume messages for `--resume <id>`.
pub(crate) fn load_resume_messages(session_id: &str) -> Result<Vec<serde_json::Value>> {
    let db_path = archon_session::storage::default_db_path();
    let store = archon_session::storage::SessionStore::open(&db_path)
        .map_err(|e| anyhow::anyhow!("failed to open session database: {e}"))?;
    let (meta, raw_messages) = archon_session::resume::resume_session(&store, session_id)
        .map_err(|e| anyhow::anyhow!("failed to resume session: {e}"))?;
    eprintln!(
        "Resumed session {} ({} messages, {} tokens)",
        &meta.id[..8.min(meta.id.len())],
        meta.message_count,
        meta.total_tokens,
    );
    // Parse stored JSON strings back into Values
    let messages: Vec<serde_json::Value> = raw_messages
        .iter()
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect();
    Ok(messages)
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
    // Reconstruct config_path and layer_filter for source tracking
    let config_path = env_vars
        .config_dir
        .as_ref()
        .map(|d| d.join("config.toml"))
        .unwrap_or_else(default_config_path);

    let layer_filter: Option<Vec<ConfigLayer>> =
        cli.setting_sources.as_ref().map(|s| parse_layer_filter(s));

    // ── Open session store and register this session ────────────
    let session_db = config
        .session
        .db_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(archon_session::storage::default_db_path);
    let session_store = Arc::new(
        archon_session::storage::SessionStore::open(&session_db)
            .map_err(|e| anyhow::anyhow!("failed to open session store: {e}"))?,
    );
    let session_store_fwd = Arc::clone(&session_store);

    // ── Phase 2: Open memory via server/client access (CLI-103, CLI-234) ──
    let data_dir = config
        .memory
        .db_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("archon")
        });
    // Keep memory_access alive for the lifetime of the session so the TCP
    // server handle (in the Direct variant) is not dropped.
    let _memory_access = archon_memory::open_memory(&data_dir)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("failed to open memory: {e}, using in-memory fallback");
            let graph = MemoryGraph::in_memory().expect("in-memory graph");
            MemoryAccess::Direct {
                graph: Arc::new(graph),
                _server_handle: tokio::spawn(async {}),
            }
        });
    // ── Phase 2b: Set up semantic embedding provider (CLI-205) ──
    // Embedding setup needs the concrete MemoryGraph (Direct mode only).
    if let Some(graph) = _memory_access.graph() {
        let embed_cfg = archon_memory::embedding::EmbeddingConfig {
            provider: config.memory.embedding_provider,
            hybrid_alpha: config.memory.hybrid_alpha,
        };
        match archon_memory::embedding::create_provider(&embed_cfg) {
            Ok(provider) => {
                if let Err(e) = graph.set_embedding_provider(provider) {
                    tracing::warn!("failed to initialise embedding schema: {e}");
                } else {
                    graph.set_hybrid_alpha(embed_cfg.hybrid_alpha);
                    tracing::info!(
                        provider = %embed_cfg.provider,
                        alpha = embed_cfg.hybrid_alpha,
                        "semantic embedding provider active"
                    );
                }
            }
            Err(e) => {
                tracing::warn!("embedding provider unavailable, using keyword-only search: {e}");
            }
        }
    }
    // Wrap MemoryAccess as Arc<dyn MemoryTrait> — works for both Direct and
    // Remote variants, no more falling back to an in-memory graph for Remote.
    let memory: Arc<dyn MemoryTrait> = Arc::new(_memory_access);
    tracing::info!("memory system opened");

    // ── Phase 2: Validate personality (CLI-105) ─────────────────
    if let Err(e) = config.personality.validate() {
        tracing::warn!("invalid personality config: {e}, using defaults");
    }

    // ── Phase 2: Load behavioral rules + defaults (CLI-106) ─────
    let rules_engine = RulesEngine::new(memory.as_ref());
    match load_configured_defaults(&rules_engine, &config.consciousness.initial_rules) {
        Ok(n) if n > 0 => tracing::info!("loaded {n} default behavioral rules"),
        Ok(_) => tracing::debug!("behavioral rules already present"),
        Err(e) => tracing::warn!("failed to load default rules: {e}"),
    }

    // ── CRIT-15: Auto-update check at startup ─────────────────────
    if archon_core::update::should_auto_check(&config.update) {
        let update_config = config.update.clone();
        tokio::spawn(async move {
            match archon_core::update::check_update(&update_config).await {
                Ok(msg) => tracing::info!("auto-update check: {msg}"),
                Err(e) => tracing::debug!("auto-update check: {e}"),
            }
            archon_core::update::record_check_time();
        });
    }

    // ── Phase 2: Initialize fast mode (CLI-118) ─────────────────
    // Shared atomic so slash commands and the agent see the same state
    let fast_mode_shared = Arc::new(AtomicBool::new(cli.fast));
    let fast_mode = FastModeState::new_with(cli.fast);
    if cli.fast {
        tracing::info!("fast mode enabled via --fast flag");
    }

    // ── Phase 2: Initialize effort state (CLI-119) ──────────────
    // Default effort is Medium; ultrathink keyword bumps to High per-turn.
    let mut initial_effort = EffortLevel::Medium;
    let mut effort_state = EffortState::new();
    if let Some(ref effort_arg) = cli.effort {
        match effort::parse_level(effort_arg) {
            Ok(level) => {
                effort_state.set_level(level);
                initial_effort = level;
                tracing::info!("effort level set to {level} via --effort flag");
            }
            Err(e) => {
                tracing::warn!("invalid --effort value: {e}, using default (medium)");
            }
        }
    } else {
        // Use config default (medium unless overridden in config)
        match effort::parse_level(&config.api.default_effort) {
            Ok(level) => {
                effort_state.set_level(level);
                initial_effort = level;
            }
            Err(_) => {} // default is Medium, already set
        }
    }
    let effort_level_shared = Arc::new(tokio::sync::Mutex::new(initial_effort));

    // ── Phase 2: Initialize model override state ────────────────
    let model_override_shared = Arc::new(tokio::sync::Mutex::new(String::new()));

    // ── Phase 2: Initialize cost alert state (CLI-122) ──────────
    let cost_alert_state = CostAlertState::new(&config.cost);
    tracing::debug!(
        "cost alerts: warn={}, hard={}",
        config.cost.warn_threshold,
        config.cost.hard_limit
    );

    // ── Phase 2: Open checkpoint store (CLI-116) ────────────────
    let checkpoint_db_path = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("archon")
        .join("checkpoints.db");
    let checkpoint_store = if config.checkpoint.enabled {
        match archon_session::checkpoint::CheckpointStore::open_with_limit(
            &checkpoint_db_path,
            config.checkpoint.max_checkpoints,
        ) {
            Ok(store) => {
                tracing::info!(
                    "checkpoint store opened at {}",
                    checkpoint_db_path.display()
                );
                Some(store)
            }
            Err(e) => {
                tracing::warn!("failed to open checkpoint store: {e}, checkpoints disabled");
                None
            }
        }
    } else {
        tracing::debug!("checkpoints disabled in config");
        None
    };

    // ── Phase 2: Load MCP server configs (CLI-101) ──────────────
    let working_dir = std::env::current_dir().unwrap_or_default();
    let mcp_configs = if resolved_flags.bare_mode {
        tracing::info!("bare mode: skipping MCP auto-discovery");
        Vec::new()
    } else {
        archon_mcp::config::load_merged_configs(&working_dir).unwrap_or_else(|e| {
            tracing::warn!("failed to load MCP configs: {e}");
            Vec::new()
        })
    };

    // Start MCP servers synchronously so tools are available before building AgentConfig.
    // A 15-second timeout ensures a slow or absent MCP server never hangs startup.
    let mcp_manager = archon_mcp::lifecycle::McpServerManager::new();
    let mcp_tools: Vec<archon_mcp::tool_bridge::McpTool> = if !mcp_configs.is_empty() {
        match tokio::time::timeout(
            std::time::Duration::from_secs(15),
            mcp_manager.start_all(mcp_configs),
        )
        .await
        {
            Ok(errors) => {
                for e in &errors {
                    tracing::warn!("MCP server start error: {e}");
                }
                if errors.is_empty() {
                    tracing::info!("all MCP servers started");
                }
            }
            Err(_) => {
                tracing::warn!(
                    "MCP server startup timed out after 15s — continuing without MCP tools"
                );
            }
        }
        mcp_manager.build_mcp_tools().await
    } else {
        Vec::new()
    };

    // ── Resolve authentication ──────────────────────────────────
    let auth = match resolve_auth_with_keys(
        env_vars.anthropic_api_key.as_deref(),
        env_vars.archon_api_key.as_deref(),
        env_vars.archon_oauth_token.as_deref(),
        std::env::var("ANTHROPIC_AUTH_TOKEN").ok().as_deref(),
    ) {
        Ok(a) => match &a {
            archon_llm::auth::AuthProvider::OAuthToken(creds) => {
                tracing::info!(
                    "authenticated via OAuth (subscription: {})",
                    creds.subscription_type
                );
                if creds.is_expired() {
                    tracing::warn!("OAuth token is expired, attempting refresh...");
                    let http = reqwest::Client::new();
                    let cred_path = archon_llm::tokens::credentials_path();
                    match archon_llm::tokens::refresh_if_needed(&cred_path, &http).await {
                        Ok(refreshed) => {
                            tracing::info!("OAuth token refreshed successfully");
                            archon_llm::auth::AuthProvider::OAuthToken(refreshed)
                        }
                        Err(e) => {
                            eprintln!("Token refresh failed: {e}");
                            eprintln!("Run `archon login` to re-authenticate.");
                            std::process::exit(1);
                        }
                    }
                } else {
                    a
                }
            }
            archon_llm::auth::AuthProvider::ApiKey(_) => {
                tracing::info!("authenticated via API key (fallback)");
                a
            }
            archon_llm::auth::AuthProvider::BearerToken(_) => {
                tracing::info!("authenticated via bearer token");
                a
            }
        },
        Err(e) => {
            eprintln!("Authentication failed: {e}");
            eprintln!("Run `archon login` or set ANTHROPIC_API_KEY.");
            std::process::exit(1);
        }
    };

    // Build identity provider
    let device_id = get_or_create_device_id();
    let betas = resolve_betas(config.identity.spoof_betas.as_deref());

    let identity_mode = if cli.identity_spoof {
        // --identity-spoof flag overrides everything
        IdentityMode::Spoof {
            version: config.identity.spoof_version.clone(),
            entrypoint: config.identity.spoof_entrypoint.clone(),
            betas,
            workload: config.identity.workload.clone(),
            anti_distillation: config.identity.anti_distillation,
        }
    } else {
        match config.identity.mode.as_str() {
            "clean" => IdentityMode::Clean,
            "custom" => {
                let custom = config.identity.custom.as_ref();
                IdentityMode::Custom {
                    user_agent: custom.map(|c| c.user_agent.clone()).unwrap_or_else(|| {
                        concat!("archon-cli/", env!("CARGO_PKG_VERSION")).into()
                    }),
                    x_app: custom
                        .map(|c| c.x_app.clone())
                        .unwrap_or_else(|| "archon".into()),
                    extra_headers: custom
                        .and_then(|c| c.extra_headers.clone())
                        .unwrap_or_default(),
                }
            }
            "spoof" => IdentityMode::Spoof {
                version: config.identity.spoof_version.clone(),
                entrypoint: config.identity.spoof_entrypoint.clone(),
                betas: resolve_betas(config.identity.spoof_betas.as_deref()),
                workload: config.identity.workload.clone(),
                anti_distillation: config.identity.anti_distillation,
            },
            _ => IdentityMode::Clean,
        }
    };

    tracing::debug!(
        "Identity mode: {}",
        match &identity_mode {
            IdentityMode::Clean => "clean",
            IdentityMode::Spoof { .. } => "spoof",
            IdentityMode::Custom { .. } => "custom",
        }
    );

    // Fetch account_uuid from OAuth profile
    let account_uuid = fetch_account_uuid(&auth).await;

    let identity = IdentityProvider::new(
        identity_mode,
        session_id.to_string(),
        device_id,
        account_uuid,
    );

    // Resolve API base URL: env var > config > hardcoded default (inside AnthropicClient::new)
    let api_url = std::env::var("ANTHROPIC_BASE_URL")
        .ok()
        .or_else(|| config.api.base_url.clone());

    // Create API client (clone auth/identity for /btw side questions)
    let btw_auth = auth.clone();
    let btw_identity = identity.clone();
    let api_client = AnthropicClient::new(auth, identity.clone(), api_url.clone());

    // In spoof mode without explicit betas: background-discover and validate betas for next startup.
    // We spawn this AFTER building the client so the probe uses the same auth.
    // The current session uses the betas already resolved above; the validated cache will be
    // used on the NEXT startup, ensuring the probe never blocks interactive startup.
    if matches!(identity.mode, IdentityMode::Spoof { .. }) && config.identity.spoof_betas.is_none()
    {
        let client_for_discovery = api_client.clone();
        tokio::spawn(async move {
            let validated = resolve_and_validate_betas(&client_for_discovery, None).await;
            tracing::info!(
                "Background beta discovery complete: {} betas validated",
                validated.len()
            );
        });
    }

    // Build tool registry and get tool definitions for API
    let mut registry = create_default_registry(working_dir.clone());
    // Wire config-driven bash tool limits
    registry.register(Box::new(archon_tools::bash::BashTool {
        timeout_secs: config.tools.bash_timeout,
        max_output_bytes: config.tools.bash_max_output,
    }));

    // Apply tool filtering from resolved flags (CLI-220)
    apply_tool_filters(&mut registry, resolved_flags);

    // ── Fix 2: Load and instantiate WASM plugins, inject their tools ──────────
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

    // ── Fix 4: Spawn cron scheduler background task ───────────────────────────
    let _cron_cancel = {
        let cron_store_path = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("archon")
            .join("scheduled_tasks.json");
        let cancel = Arc::new(AtomicBool::new(false));
        tokio::spawn(archon_tools::cron_scheduler::run_scheduler_loop(
            cron_store_path,
            Arc::clone(&cancel),
        ));
        cancel
    };

    // Register memory tools backed by the CozoDB graph — gated by config.memory.enabled (TASK-WIRE-002)
    if config.memory.enabled {
        registry.register(Box::new(archon_tools::memory::MemoryStoreTool::new(
            Arc::clone(&memory),
        )));
        registry.register(Box::new(archon_tools::memory::MemoryRecallTool::new(
            Arc::clone(&memory),
        )));
    }

    // ── VerbosityToggle (CLI-314) ──────────────────────────────
    // Initial verbosity comes from config (default: true = verbose).
    let verbosity_state = std::sync::Arc::new(std::sync::Mutex::new(
        archon_tools::verbosity_toggle::VerbosityState::new(config.tui.verbose),
    ));
    registry.register(Box::new(
        archon_tools::verbosity_toggle::VerbosityToggleTool::new(Arc::clone(&verbosity_state)),
    ));

    // Register MCP tools into the agent registry so the LLM sees them in tool_defs.
    let mcp_tool_count = mcp_tools.len();
    for tool in mcp_tools {
        registry.register(Box::new(tool));
    }
    if mcp_tool_count > 0 {
        tracing::info!("registered {mcp_tool_count} MCP tools into agent registry");
    }

    // ── Resolve --agent flag against AgentRegistry (AGT-008) ──
    // Load a temporary registry for resolution; the Arc-wrapped one is created later.
    let agent_registry_tmp = AgentRegistry::load(&working_dir);

    // ── Inject agent listing into Agent tool description (AGT-011) ──
    {
        let agents: Vec<(String, String)> = agent_registry_tmp
            .list()
            .iter()
            .map(|a| (a.agent_type.clone(), a.description.clone()))
            .collect();
        registry.register(Box::new(
            archon_tools::agent_tool::AgentTool::with_agent_listing(&agents),
        ));
    }

    let agent_def: Option<archon_core::agents::CustomAgentDefinition> =
        if let Some(ref agent_name) = resolved_flags.agent {
            match agent_registry_tmp.resolve(agent_name) {
                Some(def) => {
                    tracing::info!(agent = agent_name, "resolved custom agent definition");
                    Some(def.clone())
                }
                None => {
                    let available = agent_registry_tmp.available_agent_names().join(", ");
                    anyhow::bail!("Unknown agent '{}'. Available: {}", agent_name, available);
                }
            }
        } else {
            None
        };
    drop(agent_registry_tmp);

    // Apply agent tool filtering to registry
    if let Some(ref def) = agent_def {
        if let Some(ref allowed) = def.allowed_tools {
            let allowed_refs: Vec<&str> = allowed.iter().map(|s: &String| s.as_str()).collect();
            registry.filter_whitelist(&allowed_refs);
        }
        if let Some(ref denied) = def.disallowed_tools {
            let denied_refs: Vec<&str> = denied.iter().map(|s: &String| s.as_str()).collect();
            registry.filter_blacklist(&denied_refs);
        }
    }

    // Pre-flight check: required MCP servers must be available for --agent mode
    if let Some(ref def) = agent_def {
        let available_tools = registry.tool_names();
        let available_mcp: Vec<String> = available_tools
            .iter()
            .filter(|n| n.starts_with("mcp__"))
            .map(|n| n.to_string())
            .collect();
        if !def.has_required_mcp_servers(&available_mcp) {
            anyhow::bail!(
                "Agent '{}' requires MCP servers {:?} but they are not available.",
                def.agent_type,
                def.required_mcp_servers,
            );
        }
    }

    let tool_defs = registry.tool_definitions();

    // ── Phase 2: Assemble system prompt with consciousness (CLI-108) ──
    // Note: omit_claude_md is subagent-spawn only (matches Claude Code behavior).
    // --agent CLI mode always gets full ARCHON.md.
    let archon_md = if resolved_flags.bare_mode {
        tracing::info!("bare mode: skipping ARCHON.md loading");
        String::new()
    } else {
        archon_core::archonmd::load_hierarchical_archon_md_with_limit(
            &working_dir,
            config.context.archonmd_max_tokens as usize,
        )
    };
    let git_info = archon_core::git::detect_git_info(&working_dir);
    let git_branch = git_info.as_ref().map(|g| g.branch.as_str());
    let env_section = build_environment_section(&working_dir, git_branch);

    // Register this session in the session store
    if let Err(e) = session_store.register_session(
        session_id,
        &working_dir.display().to_string(),
        git_branch,
        &config.api.default_model,
    ) {
        tracing::warn!("failed to register session: {e}");
    }

    // Wire --session-name: assign a human-readable name at startup
    if let Some(ref name) = cli.session_name {
        if let Err(e) = archon_session::naming::set_session_name(&session_store, session_id, name) {
            tracing::warn!("failed to set session name: {e}");
        } else {
            tracing::info!("session named: {name}");
        }
    }

    // Build identity blocks as a text string for the assembler
    let identity_blocks = identity.system_prompt_blocks("", &archon_md, &env_section);
    let identity_text = identity_blocks
        .iter()
        .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join("\n\n");

    let personality_text = config.personality.to_prompt_text();
    let rules_text = rules_engine.format_for_prompt().unwrap_or_default();

    let assembler = SystemPromptAssembler::new(BudgetConfig::default());
    let input = AssemblyInput {
        identity: if identity_text.is_empty() {
            None
        } else {
            Some(identity_text)
        },
        personality: if agent_def.is_some() || resolved_flags.bare_mode {
            None
        } else {
            Some(personality_text)
        },
        rules: if rules_text.is_empty() {
            None
        } else {
            Some(rules_text)
        },
        memories: None, // Memory injection on first turn is empty (no context yet)
        user_prompt: None,
        project_instructions: if archon_md.is_empty() {
            None
        } else {
            Some(archon_md.clone())
        },
        environment: if env_section.is_empty() {
            None
        } else {
            Some(env_section.clone())
        },
        inner_voice: None, // Populated on subsequent turns by InnerVoice::to_prompt_block()
        personality_briefing: agent_def.as_ref().map(|a| {
            let mut prompt = a.system_prompt.clone();
            // Inject tool guidance into agent system prompt
            if !a.tool_guidance.is_empty() {
                prompt = format!("{prompt}\n\n<tool-guidance>\n{}\n</tool-guidance>", a.tool_guidance);
            }
            // Inject skills into agent system prompt
            if let Some(ref skills) = a.skills {
                if !skills.is_empty() {
                    let skills_list = skills.join(", ");
                    prompt = format!("{prompt}\n\n<available-skills>\nThe following skills are available to you: {skills_list}\nInvoke them by name when relevant to the task.\n</available-skills>");
                }
            }
            // Inject LEANN queries and memory tags
            if !a.leann_queries.is_empty() {
                let queries = a.leann_queries.join(", ");
                prompt = format!("{prompt}\n\n<leann-queries>\nRelevant code search queries for your task: {queries}\nUse these with the LEANN semantic search tool when exploring the codebase.\n</leann-queries>");
            }
            if !a.tags.is_empty() {
                let tags = a.tags.join(", ");
                prompt = format!("{prompt}\n\n<agent-tags>\nYour memory tags: {tags}\nUse these tags when storing or recalling memories relevant to your role.\n</agent-tags>");
            }
            prompt
        }),
        memory_briefing: None, // Injected on first turn via agent field
        dynamic: Some(format!(
            "Date: {}\nSession: {}\n\n\
            ## Memory System\n\
            You have a persistent memory graph backed by CozoDB. Use it proactively:\n\
            - `memory_store`: Save facts, decisions, preferences, and behavioral rules for future recall. \
            Always store things the user asks you to remember.\n\
            - `memory_recall`: Search past memories by keyword. Use this when the user asks what you \
            remember, or when context from past sessions would be useful.\n\
            Memories persist across sessions. Store important decisions and preferences immediately.",
            chrono::Utc::now().format("%Y-%m-%d"),
            session_id
        )),
    };

    let sections = assembler.assemble(&input);

    // Convert assembled sections into Vec<serde_json::Value> for the API
    let system_prompt: Vec<serde_json::Value> = if let Some(ref override_text) =
        resolved_flags.system_prompt_override
    {
        // --system-prompt or --system-prompt-file: replace the entire prompt
        vec![serde_json::json!({ "type": "text", "text": override_text })]
    } else {
        let mut blocks: Vec<serde_json::Value> = sections
            .into_iter()
            .map(|section| {
                let mut block = serde_json::json!({
                    "type": "text",
                    "text": section.content,
                });
                // Gated by config.context.prompt_cache (TASK-WIRE-003) — when
                // disabled, we omit the cache_control hint entirely so the API
                // treats every block as non-cacheable.
                if config.context.prompt_cache {
                    if let Some(ref cc) = section.cache_control {
                        block["cache_control"] = serde_json::json!({ "type": cc });
                    }
                }
                block
            })
            .collect();
        // --append-system-prompt or --append-system-prompt-file: append to assembled prompt
        if let Some(ref append_text) = resolved_flags.system_prompt_append {
            blocks.push(serde_json::json!({ "type": "text", "text": append_text }));
        }

        // ── Output style injection (CLI-310) ─────────────────────
        // CLI flag takes priority over config.toml value.
        // If neither is set, a plugin-forced style is used as fallback.
        {
            use archon_core::output_style::OutputStyleRegistry;
            use archon_core::output_style_loader::load_styles_from_dir;

            let mut reg = OutputStyleRegistry::new();
            if let Some(home) = dirs::home_dir() {
                let new_dir = home.join(".archon").join("output-styles");
                if new_dir.is_dir() {
                    for style in load_styles_from_dir(&new_dir) {
                        reg.register(style);
                    }
                } else {
                    let old_dir = home.join(".claude").join("output-styles");
                    if old_dir.is_dir() {
                        tracing::warn!(
                            "Loading from deprecated path {}. Rename to {} to suppress this warning.",
                            old_dir.display(),
                            new_dir.display()
                        );
                        for style in load_styles_from_dir(&old_dir) {
                            reg.register(style);
                        }
                    }
                }
            }

            let style_name: Option<&str> = cli
                .output_style
                .as_deref()
                .or(config.output_style.as_deref());

            let injection = if let Some(name) = style_name {
                let style = reg.get_or_default(name);
                style.prompt.clone()
            } else {
                reg.forced_plugin_style().and_then(|s| s.prompt.clone())
            };

            if let Some(ref text) = injection {
                tracing::info!("injecting output style into system prompt");
                blocks.push(serde_json::json!({ "type": "text", "text": text }));
            }
        }

        blocks
    };

    // Shared permission mode — honour CLI flags and config
    let initial_perm_mode = if cli.dangerously_skip_permissions {
        "bypassPermissions".to_string()
    } else if let Some(ref pm) = cli.permission_mode {
        pm.clone()
    } else {
        config.permissions.mode.clone()
    };
    let permission_mode_shared = Arc::new(tokio::sync::Mutex::new(initial_perm_mode));

    // Clone system prompt for /btw side questions (shares prompt cache)
    let btw_system_prompt = system_prompt.clone();

    // Pre-compute system prompt and tool definition sizes for /context display
    let system_prompt_chars: usize = system_prompt
        .iter()
        .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
        .map(|s| s.len())
        .sum();
    let tool_defs_chars: usize = tool_defs
        .iter()
        .map(|t| serde_json::to_string(t).unwrap_or_default().len())
        .sum();

    // Build agent config with shared fast_mode + effort state (GAP 3 & 4)
    let mut agent_config = AgentConfig {
        model: config.api.default_model.clone(),
        max_tokens: config.api.thinking_budget,
        thinking_budget: config.api.thinking_budget,
        system_prompt,
        tools: tool_defs,
        working_dir: working_dir.clone(),
        session_id: session_id.to_string(),
        fast_mode: Arc::clone(&fast_mode_shared),
        effort_level: Arc::clone(&effort_level_shared),
        model_override: Arc::clone(&model_override_shared),
        permission_mode: Arc::clone(&permission_mode_shared),
        extra_dirs: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        max_tool_concurrency: config.tools.max_concurrency as usize,
        max_turns: None,
        cancel_token: None,
    };

    // Apply agent execution config overrides
    if let Some(ref def) = agent_def {
        // AC-113: model="inherit" means use parent model (skip override)
        if let Some(ref m) = def.model {
            if m != "inherit" {
                agent_config.model = m.clone();
                *agent_config.model_override.blocking_lock() = m.clone();
            }
        }
        if let Some(ref e) = def.effort {
            if let Ok(level) = e.parse::<archon_llm::effort::EffortLevel>() {
                *agent_config.effort_level.blocking_lock() = level;
            } else {
                tracing::warn!(agent = %def.agent_type, effort = %e, "invalid effort level in agent definition, using default");
            }
        }
        if let Some(ref pm) = def.permission_mode {
            let mode_str = pm.as_str();
            // AC-103: Agent permission_mode must NOT override parent BypassPermissions/AcceptEdits/Auto
            let parent_mode = agent_config.permission_mode.blocking_lock().clone();
            let parent_is_privileged = matches!(
                parent_mode.as_str(),
                "bypassPermissions" | "acceptEdits" | "auto"
            );
            if parent_is_privileged {
                tracing::debug!(
                    agent = %def.agent_type, parent_mode = %parent_mode, agent_mode = %mode_str,
                    "agent permission_mode skipped — parent has privileged mode"
                );
            } else if mode_str == "bypassPermissions" && !cli.dangerously_skip_permissions {
                tracing::warn!(
                    agent = %def.agent_type, raw_mode = %pm,
                    "agent requests bypassPermissions but --dangerously-skip-permissions not passed; ignoring"
                );
            } else {
                *agent_config.permission_mode.blocking_lock() = mode_str.to_string();
            }
        }
        if def.max_turns.is_some() {
            agent_config.max_turns = def.max_turns;
        }
    }

    let extra_dirs_shared = Arc::clone(&agent_config.extra_dirs);

    // Create channels
    let (agent_event_tx, mut agent_event_rx) =
        tokio::sync::mpsc::unbounded_channel::<TimestampedEvent>();
    // TASK-SESSION-LOOP-EXTRACT (A-2): TuiEvent channel flipped to
    // `UnboundedSender<TuiEvent>`. This is the final Send-bound
    // holdout that blocked `archon-cli-workspace` bin builds on CI —
    // rustc's HRTB check could not prove `for<'a> &'a Sender<T>: Send`
    // across await suspension for the ~45 `&Sender<TuiEvent>` borrows
    // held across `.await` in `session_loop::run_session_loop`
    // (rust-lang/rust#102211). `UnboundedSender::send` is synchronous,
    // so no `&Sender` is held across any `.await` — Send is trivially
    // satisfied. Rationale: (1) consistency with `AgentEvent` which
    // is already `UnboundedSender` (spawn-everything philosophy,
    // PRD-2 D10); (2) OBS-914 10k events/sec load test never showed
    // `TuiEvent` saturation — local CLI + <1ms render = no realistic
    // producer-faster-than-consumer scenario. Follow-up #218
    // TUI-EVENT-BACKPRESSURE-MONITORING will add runtime channel-depth
    // metrics via the existing ChannelMetrics infra (OBS-901).
    let (tui_event_tx, tui_event_rx) = tokio::sync::mpsc::unbounded_channel::<TuiEvent>();
    // CRIT-13: Forward voice pipeline events to TUI event channel
    if let Some(mut voice_rx) = voice_event_rx {
        let voice_fwd_tx = tui_event_tx.clone();
        tokio::spawn(async move {
            while let Some(evt) = voice_rx.recv().await {
                if voice_fwd_tx.send(evt).is_err() {
                    break;
                }
            }
        });
    }
    let (user_input_tx, mut user_input_rx) = tokio::sync::mpsc::channel::<String>(16);

    // Create agent
    let provider = build_llm_provider(&config.llm, api_client);
    tracing::info!("LLM provider: {}", provider.name());

    // Load custom agent registry (built-in + project + user agents)
    let agent_registry = Arc::new(std::sync::RwLock::new(AgentRegistry::load(&working_dir)));
    {
        let reg = agent_registry.read().expect("agent registry lock");
        tracing::info!(count = reg.len(), "loaded agent definitions");
        for err in reg.load_errors() {
            tracing::warn!(%err, "agent load error");
        }
    }
    let agent_registry_for_skills = Arc::clone(&agent_registry);

    // TASK-DS-001: construct async TaskService for TUI agent/pipeline
    // invocation. Uses a separate AgentRegistry load (not the RwLock-
    // wrapped one) because DefaultTaskService::new takes Arc<AgentRegistry>,
    // not Arc<RwLock<AgentRegistry>>. Pattern matches
    // src/command/task.rs:175-177. 10000 = max queue size.
    let task_service: Arc<dyn archon_core::tasks::TaskService> =
        Arc::new(archon_core::tasks::DefaultTaskService::new(
            Arc::new(archon_core::agents::AgentRegistry::load(&working_dir)),
            10000,
        ));

    // Initialise LEANN code index for pipeline deep-search context.
    // Resilient: if the DB fails to open, leann stays None and pipelines
    // run without semantic search (same as CLI init_leann).
    let leann: Option<Arc<archon_pipeline::runner::LeannIntegration>> = {
        let db_path = working_dir.join(".archon").join("leann.db");
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match archon_leann::CodeIndex::new(&db_path, Default::default()) {
            Ok(idx) => {
                let li = Arc::new(archon_pipeline::runner::LeannIntegration::new(
                    std::sync::Arc::new(idx),
                ));
                // Init the index in the background — non-blocking.
                let li_bg = Arc::clone(&li);
                let wd = working_dir.clone();
                tokio::spawn(async move {
                    if let Err(e) = li_bg.init_repository(&wd).await {
                        tracing::warn!(error = %e, "LEANN background init failed; continuing without code context");
                    }
                });
                Some(li)
            }
            Err(e) => {
                tracing::warn!(error = %e, "LEANN unavailable; continuing without code context");
                None
            }
        }
    };

    // Construct pipeline facades + LLM adapter once at bootstrap for
    // TUI /archon-code and /archon-research commands per Deliverable 3.
    let coding_pipeline: Arc<archon_pipeline::coding::facade::CodingFacade> =
        Arc::new(archon_pipeline::coding::facade::CodingFacade::new());
    let research_pipeline: Arc<archon_pipeline::research::facade::ResearchFacade> =
        Arc::new(archon_pipeline::research::facade::ResearchFacade::new(
            Arc::clone(&memory),
            None, // LEANN searcher not wired (separate from LeannIntegration)
            working_dir.display().to_string(),
            None, // no style override
        ));
    let llm_adapter: Arc<dyn archon_pipeline::runner::LlmClient> = {
        let pipe_auth = resolve_auth_with_keys(
            env_vars.anthropic_api_key.as_deref(),
            env_vars.archon_api_key.as_deref(),
            env_vars.archon_oauth_token.as_deref(),
            std::env::var("ANTHROPIC_AUTH_TOKEN").ok().as_deref(),
        )
        .map_err(|e| tracing::warn!("Pipeline LLM auth unavailable: {e}"))
        .unwrap_or(archon_llm::auth::AuthProvider::ApiKey(
            archon_llm::types::Secret::new(String::new()),
        ));
        let identity = archon_llm::identity::IdentityProvider::new(
            archon_llm::identity::IdentityMode::Clean,
            uuid::Uuid::new_v4().to_string(),
            "tui-pipeline-device".to_string(),
            String::new(),
        );
        let api_url = std::env::var("ANTHROPIC_BASE_URL")
            .ok()
            .or_else(|| config.api.base_url.clone());
        let pipe_client = archon_llm::anthropic::AnthropicClient::new(pipe_auth, identity, api_url);
        Arc::new(archon_pipeline::llm_adapter::AnthropicLlmAdapter::new(
            Arc::new(pipe_client),
        ))
    };
    // TASK-TUI-107: clone the agent_event_tx so the dispatcher constructed
    // inside the input-loop spawn can also hold a producer. The original
    // sender is moved into Agent::new below.
    let agent_event_tx_for_dispatcher = agent_event_tx.clone();
    let mut agent = Agent::new(
        provider,
        registry,
        agent_config,
        agent_event_tx,
        agent_registry,
    );
    let metrics = Arc::new(archon_tui::observability::ChannelMetrics::default());
    let metrics_for_agent = Arc::clone(&metrics);
    agent.set_channel_metrics(metrics_for_agent);

    // TASK-TUI-803: spawn Prometheus /metrics exporter on loopback when
    // `--metrics-port <PORT>` is set (non-zero). Same Arc + synchronous
    // bind-error propagation as the print-mode path.
    spawn_metrics_exporter(cli.metrics_port, Arc::clone(&metrics))?;

    // Wire checkpoint store into agent (CLI-116)
    if let Some(store) = checkpoint_store {
        agent.set_checkpoint_store(store);
    }

    // Wire plan store into agent — shares session DB for plan persistence
    if let Ok(plan_store) = archon_session::plan::PlanStore::new(session_store.db()) {
        agent.set_plan_store(plan_store);
        tracing::info!("plan store wired into agent");
    } else {
        tracing::warn!("failed to initialize plan store");
    }

    // GAP 5/7: Wire memory graph into agent — gated by config.memory.enabled (TASK-WIRE-002)
    if config.memory.enabled {
        agent.set_memory(Arc::clone(&memory));
    }

    // Wire inner voice if enabled in config. The state is injected into
    // the system prompt before every turn and updated from tool outcomes.
    if archon_consciousness::inner_voice::InnerVoice::is_enabled(config.consciousness.inner_voice) {
        let iv = Arc::new(tokio::sync::Mutex::new(
            archon_consciousness::inner_voice::InnerVoice::with_decay_rate(
                config.consciousness.energy_decay_rate,
            ),
        ));
        agent.set_inner_voice(iv);
    }

    // CLI-416: Restore personality state from last session's PersonalitySnapshot.
    if config.consciousness.persist_personality {
        match archon_consciousness::persistence::load_latest_snapshot(memory.as_ref()) {
            Ok(Some(snap)) => {
                // Restore inner voice state from the snapshot.
                if let Some(iv_arc) = agent.inner_voice() {
                    let restored = archon_consciousness::inner_voice::InnerVoice::from_snapshot(
                        snap.inner_voice.clone(),
                    );
                    *iv_arc.lock().await = restored;
                    tracing::info!(
                        confidence = snap.inner_voice.confidence,
                        energy = snap.inner_voice.energy,
                        "personality: restored inner voice from previous session"
                    );
                }
                // Restore rule scores from the snapshot.
                let engine = archon_consciousness::rules::RulesEngine::new(memory.as_ref());
                match engine.import_scores(&snap.rule_scores) {
                    Ok(n) => tracing::info!(imported = n, "personality: restored rule scores"),
                    Err(e) => tracing::warn!("personality: failed to restore rule scores: {e}"),
                }
            }
            Ok(None) => {
                tracing::debug!("personality: no previous snapshot found (first run)");
            }
            Err(e) => {
                tracing::warn!("personality: failed to load snapshot: {e}");
            }
        }

        // Generate personality briefing for first turn.
        if let Ok(trends) = archon_consciousness::persistence::compute_trends(memory.as_ref(), 10) {
            if let Ok(Some(last)) =
                archon_consciousness::persistence::load_latest_snapshot(memory.as_ref())
            {
                if trends.total_sessions > 0 {
                    let briefing =
                        archon_consciousness::persistence::generate_briefing(&trends, &last);
                    agent.set_personality_briefing(briefing);
                    tracing::info!(
                        sessions = trends.total_sessions,
                        "personality: briefing generated for first turn"
                    );
                }
            }
        }
    }

    // CLI-417: Memory garden — auto-consolidation and briefing on session start.
    if config.memory.enabled && config.memory.garden.auto_consolidate {
        match archon_memory::garden::should_auto_consolidate(
            memory.as_ref(),
            config.memory.garden.min_hours_between_runs,
        ) {
            Ok(true) => {
                tracing::info!("garden: starting auto-consolidation");
                match archon_memory::garden::consolidate(memory.as_ref(), &config.memory.garden) {
                    Ok(report) => {
                        tracing::info!(
                            decayed = report.importance_decayed,
                            pruned = report.stale_pruned,
                            deduped = report.duplicates_merged,
                            merged = report.fragments_merged,
                            overflow = report.overflow_pruned,
                            before = report.total_memories_before,
                            after = report.total_memories_after,
                            ms = report.duration_ms,
                            "garden: consolidation complete"
                        );
                    }
                    Err(e) => tracing::warn!("garden: consolidation failed: {e}"),
                }
            }
            Ok(false) => tracing::debug!("garden: skipping — last run too recent"),
            Err(e) => tracing::warn!("garden: failed to check last run: {e}"),
        }
        // Generate memory briefing for first turn.
        match archon_memory::garden::generate_briefing(
            memory.as_ref(),
            config.memory.garden.briefing_limit,
        ) {
            Ok(briefing) if !briefing.is_empty() => {
                agent.set_memory_briefing(briefing);
                tracing::info!("garden: memory briefing generated for first turn");
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("garden: failed to generate briefing: {e}"),
        }
    }

    // Wire hook system — load hooks from all sources (settings.json + TOML)
    let hook_registry_arc = {
        let home_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let hook_registry = archon_core::hooks::HookRegistry::load_all(&working_dir, &home_dir);
        let arc = std::sync::Arc::new(hook_registry);
        agent.set_hook_registry(Arc::clone(&arc));
        arc
    };

    // Wire Phase G agent definition fields (hooks, critical_system_reminder)
    if let Some(ref def) = agent_def {
        // Register agent-specific hooks as session-scoped hooks
        if let Some(ref hooks_json) = def.hooks {
            match archon_core::agents::loader::parse_agent_hooks(hooks_json) {
                Ok(hook_pairs) => {
                    for (event, config) in hook_pairs {
                        hook_registry_arc.register_session_hook(session_id, event, config);
                    }
                    tracing::info!(agent = %def.agent_type, "registered agent session-scoped hooks");
                }
                Err(e) => {
                    tracing::warn!(agent = %def.agent_type, error = %e, "failed to parse agent hooks")
                }
            }
        }
        // Set critical system reminder for per-turn injection
        if let Some(ref reminder) = def.critical_system_reminder {
            agent.set_critical_system_reminder(reminder.clone());
        }
    }

    // GAP 6: Wire auto-mode evaluator
    let auto_eval = AutoModeEvaluator::new(AutoModeConfig {
        project_dir: Some(working_dir.clone()),
        ..Default::default()
    });
    agent.set_auto_evaluator(auto_eval);

    // Wire subagent executor (TASK-AGS-105) — must be AFTER all post-construction
    // setters so AgentSubagentExecutor captures hook_registry, memory, etc.
    agent.install_subagent_executor();

    // Permission prompt channel — agent waits for y/n, TUI sends response
    let (perm_prompt_tx, perm_prompt_rx) = tokio::sync::mpsc::channel::<bool>(1);
    agent.permission_response_rx = Some(Arc::new(tokio::sync::Mutex::new(perm_prompt_rx)));

    // Restore conversation from a previous session if --resume <id> was given
    if let Some(messages) = resume_messages {
        let count = messages.len();
        agent.restore_conversation(messages);
        tracing::info!("restored {count} messages from previous session");
        // Restore session name if the resumed session had one
        if let Some(Some(ref resume_id)) = cli.resume
            && let Ok(meta) = session_store.get_session(resume_id)
            && let Some(name) = meta.name
        {
            let _ = tui_event_tx.send(TuiEvent::SessionRenamed(name));
        }
        // CRIT-15 (ITEM 5): Restore inner voice from snapshot on session resume.
        if archon_consciousness::inner_voice::InnerVoice::is_enabled(
            config.consciousness.inner_voice,
        ) {
            if let Ok(memories) = memory.recall_memories("inner_voice_snapshot", 1) {
                if let Some(m) = memories.first() {
                    if let Ok(snapshot) = serde_json::from_str::<
                        archon_consciousness::inner_voice::InnerVoiceSnapshot,
                    >(&m.content)
                    {
                        let iv = Arc::new(tokio::sync::Mutex::new(
                            archon_consciousness::inner_voice::InnerVoice::from_snapshot(snapshot),
                        ));
                        agent.set_inner_voice(iv);
                        tracing::info!("inner voice state restored from snapshot");
                    }
                }
            }
        }
    }

    // Wire --fork-session: fork the resumed session so new messages go to a fresh session
    if cli.fork_session && cli.resume.is_some() {
        let fork_name = cli.session_name.as_deref();
        match archon_session::fork::fork_session(&session_store, session_id, fork_name) {
            Ok(new_id) => {
                eprintln!("Forked session as: {}", &new_id[..8.min(new_id.len())]);
            }
            Err(e) => {
                tracing::warn!("fork-session failed: {e}");
            }
        }
    }

    // GAP 2: Shared show_thinking flag
    let show_thinking = Arc::clone(&agent.show_thinking);

    // Shared session stats for /status and /cost commands
    let session_stats_shared = Arc::clone(&agent.session_stats);

    // ── Phase 2: Wrap shared state for the event forwarder (CLI-122) ──
    let config_cost = config.cost.clone();
    let mut cost_alert_state = cost_alert_state;

    // Spawn agent event forwarder (AgentEvent -> TuiEvent) with cost alert checks
    let tui_tx = tui_event_tx.clone();
    // Thinking filtering now happens in the TUI (App.show_thinking), not here.
    // Start config file watcher for hot-reload
    {
        let config_paths = vec![config_path.clone()];
        match archon_core::config_watcher::ConfigWatcher::start(&config_paths) {
            Ok(watcher) => {
                let reloader = archon_core::config_watcher::DebouncedReloader::new(
                    watcher,
                    500,
                    config.clone(),
                );
                let watch_tui_tx = tui_event_tx.clone();
                let watch_config_paths = config_paths;
                let watch_hook_registry = Arc::clone(&hook_registry_arc);
                let watch_working_dir = working_dir.clone();
                let watch_session_id = session_id.to_string();
                tokio::spawn(async move {
                    let mut reloader = reloader;
                    loop {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        if let Some(changed_keys) = reloader.check_and_reload(&watch_config_paths) {
                            // CRIT-06: Fire ConfigChange hook
                            if !changed_keys.is_empty() {
                                watch_hook_registry
                                    .execute_hooks(
                                        archon_core::hooks::HookEvent::ConfigChange,
                                        serde_json::json!({
                                            "hook_event": "ConfigChange",
                                            "changed_keys": changed_keys,
                                        }),
                                        &watch_working_dir,
                                        &watch_session_id,
                                    )
                                    .await;
                            }

                            let non_reloadable =
                                archon_core::config_diff::non_reloadable_changes(&changed_keys);
                            if !non_reloadable.is_empty() {
                                let msg = format!(
                                    "\nConfig reloaded. Non-reloadable changes (require restart): {}\n",
                                    non_reloadable.join(", ")
                                );
                                let _ = watch_tui_tx.send(TuiEvent::TextDelta(msg));
                            } else if !changed_keys.is_empty() {
                                let msg =
                                    format!("\nConfig reloaded: {}\n", changed_keys.join(", "));
                                let _ = watch_tui_tx.send(TuiEvent::TextDelta(msg));
                            }
                        }
                    }
                });
                tracing::debug!("config file watcher started");
            }
            Err(e) => {
                tracing::warn!("failed to start config watcher: {e}");
            }
        }
    }

    // BUG 4 FIX: Read from session_stats instead of independent counters so /clear resets cost alerts
    let session_stats_for_fwd = Arc::clone(&session_stats_shared);
    let session_id_fwd = session_id.to_string();
    let session_store_for_fwd = Arc::clone(&session_store_fwd);
    let last_assistant_response_shared: Arc<tokio::sync::Mutex<String>> =
        Arc::new(tokio::sync::Mutex::new(String::new()));
    let last_response_for_fwd = Arc::clone(&last_assistant_response_shared);
    // TASK-AGS-103: consumer-side back-pressure.
    // Producers push into an unbounded channel (TASK-AGS-102); here we
    // bound the in-TUI buffer through an EventCoalescer so a slow render
    // loop cannot accumulate unbounded memory. State events are preserved;
    // Progress events are dropped oldest-first once the buffer exceeds
    // SOFT_CAP / HARD_CAP. Up to RENDER_EVENT_BUDGET events are drained
    // per tick to keep the forwarder responsive.
    use archon_cli_workspace::event_coalescer::{EventCoalescer, RENDER_EVENT_BUDGET};
    tokio::spawn(async move {
        let mut coalescer = EventCoalescer::with_defaults();
        loop {
            // Block for at least one event; if the channel is closed, exit.
            let timestamped = match agent_event_rx.recv().await {
                Some(ts) => ts,
                None => break,
            };
            let elapsed_ms = (timestamped.sent_at.elapsed().as_millis() as u64).max(1);
            metrics.record_latency_ms(elapsed_ms);
            coalescer.push(timestamped.inner);
            // Drain any further pending events up to the per-tick budget.
            let mut drained = 1usize;
            while drained < RENDER_EVENT_BUDGET {
                match agent_event_rx.try_recv() {
                    Ok(ts) => {
                        let elapsed = (ts.sent_at.elapsed().as_millis() as u64).max(1);
                        metrics.record_latency_ms(elapsed);
                        coalescer.push(ts.inner);
                        drained += 1;
                    }
                    Err(_) => break,
                }
            }
            // Record drained batch size for channel observability (TASK-TUI-206)
            metrics.record_drained(drained as u64);
            // Rate-limited backlog WARN if over 10_000 (TASK-TUI-206 fix-forward)
            let _ = metrics.warn_if_backlog_over(10_000);
            // Forward coalesced events to the TUI.
            while let Some(event) = coalescer.pop() {
                let tui_event = match event {
                    AgentEvent::TextDelta(text) => {
                        // Track last assistant response for /copy
                        let mut resp = last_response_for_fwd.lock().await;
                        resp.push_str(&text);
                        TuiEvent::TextDelta(text)
                    }
                    // Always forward thinking deltas so the timer is accurate.
                    // The TUI decides whether to display or accumulate the text.
                    AgentEvent::ThinkingDelta(text) => TuiEvent::ThinkingDelta(text),
                    AgentEvent::ToolCallStarted { name, id } => TuiEvent::ToolStart { name, id },
                    AgentEvent::ToolCallComplete { name, id, result } => TuiEvent::ToolComplete {
                        name,
                        id,
                        success: !result.is_error,
                        output: result.content,
                    },
                    AgentEvent::TurnComplete {
                        input_tokens,
                        output_tokens,
                    } => {
                        // Freeze last_assistant_response (new turn will start fresh)
                        // Don't clear here — /copy should get the completed response
                        // Read cumulative tokens from shared session_stats (resets on /clear)
                        let estimated_cost = {
                            let stats = session_stats_for_fwd.lock().await;
                            (stats.input_tokens as f64 * 3.0 + stats.output_tokens as f64 * 15.0)
                                / 1_000_000.0
                        };

                        // Check cost alerts (CLI-122)
                        match cost_alert_state.check_cost(estimated_cost, &config_cost) {
                            CostAlertAction::Warn(msg) => {
                                let _ =
                                    tui_tx.send(TuiEvent::Error(format!("COST WARNING: {msg}")));
                            }
                            CostAlertAction::HardLimitPause(msg) => {
                                let _ = tui_tx.send(TuiEvent::Error(format!("COST LIMIT: {msg}")));
                            }
                            CostAlertAction::None => {}
                        }

                        // Update session store with cumulative usage
                        {
                            let stats = session_stats_for_fwd.lock().await;
                            let _ = session_store_for_fwd.update_usage(
                                &session_id_fwd,
                                stats.input_tokens + stats.output_tokens,
                                estimated_cost,
                            );
                        }

                        TuiEvent::TurnComplete {
                            input_tokens,
                            output_tokens,
                        }
                    }
                    AgentEvent::Error(msg) => TuiEvent::Error(msg),
                    AgentEvent::SessionComplete => TuiEvent::Done,
                    AgentEvent::PermissionRequired { tool, description } => {
                        TuiEvent::PermissionPrompt { tool, description }
                    }
                    AgentEvent::PermissionGranted { .. } | AgentEvent::PermissionDenied { .. } => {
                        continue;
                    }
                    _ => continue,
                };
                if tui_tx.send(tui_event).is_err() {
                    return;
                }
            }
        }
    });

    // ── Phase 2: Slash command state (CLI-110, CLI-118, CLI-119) ──
    // TASK-SESSION-LOOP-EXTRACT: `fast_mode` + `effort_state` move into
    // `run_session_loop` by value (they were mutated in the original
    // spawn block via `&mut`). No shadow-rebinding needed here.

    // Determine auth label for /doctor
    let auth_label = match resolve_auth_with_keys(
        env_vars.anthropic_api_key.as_deref(),
        env_vars.archon_api_key.as_deref(),
        env_vars.archon_oauth_token.as_deref(),
        std::env::var("ANTHROPIC_AUTH_TOKEN").ok().as_deref(),
    ) {
        Ok(archon_llm::auth::AuthProvider::OAuthToken(_)) => "OAuth".to_string(),
        Ok(archon_llm::auth::AuthProvider::ApiKey(_)) => "API key".to_string(),
        Ok(archon_llm::auth::AuthProvider::BearerToken(_)) => "Bearer token".to_string(),
        Err(_) => "none".to_string(),
    };

    // TASK-AGS-622: construct the shared command registry exactly once at App level.
    let registry: std::sync::Arc<crate::command::registry::Registry> =
        std::sync::Arc::new(crate::command::registry::default_registry());

    // TASK-AGS-623: construct the dispatcher over the shared registry.
    // PATH A hybrid — dispatcher runs as a gate at the top of
    // `handle_slash_command` while the legacy inline match continues to
    // execute the actual command bodies.
    let dispatcher: std::sync::Arc<crate::command::dispatcher::Dispatcher> = std::sync::Arc::new(
        crate::command::dispatcher::Dispatcher::new(std::sync::Arc::clone(&registry)),
    );

    let mut cmd_ctx = SlashCommandContext {
        fast_mode_shared: Arc::clone(&fast_mode_shared),
        effort_level_shared: Arc::clone(&effort_level_shared),
        model_override_shared: Arc::clone(&model_override_shared),
        default_model: config.api.default_model.clone(),
        show_thinking: Arc::clone(&show_thinking),
        session_stats: Arc::clone(&session_stats_shared),
        permission_mode: Arc::clone(&permission_mode_shared),
        session_id: session_id.to_string(),
        cost_config: config.cost.clone(),
        memory: Arc::clone(&memory),
        garden_config: config.memory.garden.clone(),
        mcp_manager: mcp_manager.clone(),
        working_dir: working_dir.clone(),
        extra_dirs: Arc::clone(&extra_dirs_shared),
        auth_label,
        config_path: config_path.clone(),
        env_vars: env_vars.clone(),
        config_sources: archon_core::config_source::ConfigSourceMap::from_layered_load(
            Some(&config_path),
            &working_dir,
            cli.settings.as_deref(),
            layer_filter.as_deref(),
        )
        .unwrap_or_default(),
        skill_registry: Arc::new({
            let mut reg = register_builtins();
            for skill in discover_user_skills(&working_dir) {
                tracing::debug!("discovered user skill: {}", skill.name);
                reg.register(Box::new(skill));
            }
            // Common aliases for built-in skills.
            // NOTE TASK-#206 SLASH-EXIT: the previous `q -> exit` skill
            // alias here was dead code — no `exit` skill ever existed in
            // the SkillRegistry. The `/q` alias is now declared on
            // `ExitHandler::aliases()` in the COMMAND registry, so /q
            // resolves to the real shutdown handler.
            reg.register_alias("?", "help");
            reg
        }),
        last_assistant_response: Arc::clone(&last_assistant_response_shared),
        system_prompt_chars,
        tool_defs_chars,
        allow_bypass_permissions: cli.allow_dangerously_skip_permissions
            || cli.dangerously_skip_permissions,
        denial_log: Arc::clone(&agent.denial_log),
        agent_registry: Arc::clone(&agent_registry_for_skills),
        task_service: Arc::clone(&task_service),
        coding_pipeline: Arc::clone(&coding_pipeline),
        research_pipeline: Arc::clone(&research_pipeline),
        llm_adapter: Arc::clone(&llm_adapter),
        leann: leann.clone(),
        registry: std::sync::Arc::clone(&registry),
        dispatcher: std::sync::Arc::clone(&dispatcher),
        // TASK-AGS-POST-6-EXPORT-MIGRATE: SIDECAR-SLOT shared slot for
        // /export. The sync handler writes an ExportDescriptor here;
        // the drain block inside the `if handled {` branch of the
        // input-processor task reads it back out and performs the
        // mutex-requiring conversation-state read + file-write I/O.
        // Initial value is None — no export queued until the handler
        // stashes one.
        pending_export_shared: Arc::new(std::sync::Mutex::new(None)),
    };

    // Spawn agent input processor with slash command dispatch
    let input_tui_tx = tui_event_tx.clone();
    let slash_commands_disabled = resolved_flags.disable_slash_commands;
    let session_store_for_input = Arc::clone(&session_store);
    let session_id_for_input = session_id.to_string();
    // CLI-416: Capture personality persistence config for session-end save.
    let persist_personality = config.consciousness.persist_personality;
    let personality_history_limit = config.consciousness.personality_history_limit;
    let session_start_instant = std::time::Instant::now();
    let session_start_confidence = if let Some(iv_arc) = agent.inner_voice() {
        iv_arc.lock().await.confidence
    } else {
        0.7
    };
    // Clone api_url for the btw_tx background task (line ~1683); the spawn below consumes it.
    let api_url_for_btw = api_url.clone();
    // TASK-TUI-107: old `handle.await`-prior serialization slot deleted.
    // Turn lifecycle now lives in `AgentDispatcher` (constructed below).
    //
    // TASK-SESSION-LOOP-EXTRACT (Path #1, channel decoupling):
    // spawn the MCP lifecycle task on a dedicated OS thread. Its
    // non-Send `connect_server()` futures live INSIDE that thread's
    // current-thread runtime and never cross the session-loop
    // `tokio::spawn` boundary. The channel handle (`McpLifecycleTx`)
    // is `Send + 'static`, so it passes through cleanly.
    let mcp_lifecycle_tx = crate::session_loop::spawn_mcp_lifecycle_task(mcp_manager.clone());
    tokio::spawn(crate::session_loop::run_session_loop(
        agent,
        agent_def,
        api_url,
        input_tui_tx,
        user_input_rx,
        agent_event_tx_for_dispatcher,
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
    ));

    // Build splash screen config with recent activity from session store
    let activity = {
        let db_path = archon_session::storage::default_db_path();
        match archon_session::storage::SessionStore::open(&db_path) {
            Ok(store) => {
                let cwd = working_dir.display().to_string();
                store
                    .list_sessions(10)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|s| s.working_directory == cwd)
                    .filter(|s| s.id != session_id)
                    .take(3)
                    .map(|s| {
                        let when = archon_tui::splash::format_relative_time(&s.last_active);
                        let msgs = s.message_count;
                        let desc = if msgs == 0 {
                            "Empty session".to_string()
                        } else {
                            format!("{msgs} messages, {}", s.model)
                        };
                        archon_tui::splash::ActivityEntry {
                            when,
                            description: desc,
                        }
                    })
                    .collect()
            }
            Err(e) => {
                tracing::debug!("could not open session store for splash: {e}");
                Vec::new()
            }
        }
    };
    let splash_opt = if resolved_flags.bare_mode {
        None
    } else {
        Some(archon_tui::app::SplashConfig {
            model: config.api.default_model.clone(),
            working_dir: working_dir.display().to_string(),
            activity,
        })
    };

    // Set up /btw side question channel — uses same auth/identity/model as main agent
    let (btw_tx, mut btw_rx) = tokio::sync::mpsc::channel::<String>(8);
    {
        let btw_tui_tx = tui_event_tx.clone();
        // Clone the same client the agent uses — same auth, identity, headers
        let btw_client = AnthropicClient::new(btw_auth, btw_identity, api_url_for_btw);
        let btw_model = config.api.default_model.clone();
        let btw_max_tokens = config.api.thinking_budget;
        // Use the pre-cloned system prompt so /btw shares the prompt cache
        let btw_system_prompt = btw_system_prompt;
        tokio::spawn(async move {
            while let Some(question) = btw_rx.recv().await {
                let tui_tx = btw_tui_tx.clone();
                let client = btw_client.clone();
                let model = btw_model.clone();
                let sys_prompt = btw_system_prompt.clone();
                let max_tokens = btw_max_tokens;
                tokio::spawn(async move {
                    let wrapped = format!(
                        "<system-reminder>This is a side question from the user. Answer directly in a single response.\n\
                         You have NO tools available. This is a one-off response.\n\
                         Do NOT say \"Let me check\" or promise actions.</system-reminder>\n\n{question}"
                    );
                    // Use the same system prompt as the main agent for cache sharing
                    let request = archon_llm::anthropic::MessageRequest {
                        model,
                        max_tokens,
                        system: sys_prompt,
                        messages: vec![serde_json::json!({
                            "role": "user",
                            "content": wrapped,
                        })],
                        tools: Vec::new(), // no tools for side questions
                        thinking: None,
                        speed: None,
                        effort: None,
                    };
                    let stream_result: Result<
                        tokio::sync::mpsc::Receiver<archon_llm::streaming::StreamEvent>,
                        _,
                    > = client.stream_message(request).await;
                    match stream_result {
                        Ok(mut rx) => {
                            let mut response = String::new();
                            while let Some(event) = rx.recv().await {
                                if let archon_llm::streaming::StreamEvent::TextDelta {
                                    ref text,
                                    ..
                                } = event
                                {
                                    response.push_str(text);
                                }
                            }
                            let _ = tui_tx.send(TuiEvent::BtwResponse(response));
                        }
                        Err(e) => {
                            let _ = tui_tx.send(TuiEvent::BtwResponse(format!("Error: {e}")));
                        }
                    }
                });
            }
        });
    }

    // Apply vim mode from config before blocking on TUI
    if config.tui.vim_mode {
        let _ = tui_event_tx.send(TuiEvent::SetVimMode(true));
    }

    // Run the TUI (blocks until user quits)
    archon_tui::app::run(archon_tui::app::AppConfig {
        event_rx: tui_event_rx,
        input_tx: user_input_tx,
        splash: splash_opt,
        btw_tx: Some(btw_tx),
        permission_tx: Some(perm_prompt_tx),
    })
    .await?;

    // ── Phase 2: Graceful MCP shutdown ──────────────────────────
    mcp_manager.shutdown_all().await;
    tracing::info!("MCP servers shut down");

    Ok(())
}
