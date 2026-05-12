use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::cli_args::Cli;
use crate::command::utils::fetch_account_uuid;
use anyhow::Result;
use archon_consciousness::defaults::load_configured_defaults;
use archon_consciousness::rules::RulesEngine;
use archon_core::config::default_config_path;
use archon_core::cost_alerts::CostAlertState;
use archon_core::env_vars::ArchonEnvVars;
use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::resolve_auth_with_keys;
use archon_llm::effort::{self, EffortLevel, EffortState};
use archon_llm::fast_mode::FastModeState;
use archon_llm::identity::{
    IdentityMode, IdentityProvider, get_or_create_device_id, resolve_and_validate_betas,
    resolve_identity_mode,
};
use archon_memory::{MemoryAccess, MemoryGraph, MemoryTrait};
use archon_tui::observability;

pub(super) struct Bootstrap {
    pub config_path: PathBuf,
    pub layer_filter: Option<Vec<archon_core::config_layers::ConfigLayer>>,
    pub session_store: Arc<archon_session::storage::SessionStore>,
    pub memory: Arc<dyn MemoryTrait>,
    pub working_dir: PathBuf,
    pub hook_registry: Arc<archon_core::hooks::HookRegistry>,
    pub mcp_manager: archon_mcp::lifecycle::McpServerManager,
    pub mcp_tools: Vec<archon_mcp::tool_bridge::McpTool>,
    pub provider_override: Option<Arc<dyn archon_llm::provider::LlmProvider>>,
    pub anthropic_client: Option<AnthropicClient>,
    pub session_api_url: Option<String>,
    pub prompt_identity: IdentityProvider,
    pub fast_mode_shared: Arc<AtomicBool>,
    pub sandbox_flag: Arc<AtomicBool>,
    pub fast_mode: FastModeState,
    pub effort_state: EffortState,
    pub effort_level_shared: Arc<tokio::sync::Mutex<EffortLevel>>,
    pub model_override_shared: Arc<tokio::sync::Mutex<String>>,
    pub cost_alert_state: CostAlertState,
    pub checkpoint_store: Option<archon_session::checkpoint::CheckpointStore>,
}

pub(super) async fn prepare(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    env_vars: &ArchonEnvVars,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
) -> Result<Bootstrap> {
    let config_path = env_vars
        .config_dir
        .as_ref()
        .map(|d| d.join("config.toml"))
        .unwrap_or_else(default_config_path);
    let layer_filter = cli
        .setting_sources
        .as_ref()
        .map(|s| crate::setup::parse_layer_filter(s));

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

    let data_dir = config
        .memory
        .db_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("archon")
        });
    let memory_access = archon_memory::open_memory(&data_dir)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("failed to open memory: {e}, using in-memory fallback");
            let graph = MemoryGraph::in_memory().expect("in-memory graph");
            MemoryAccess::Direct {
                graph: Arc::new(graph),
                _server_handle: tokio::spawn(async {}),
            }
        });
    if let Some(graph) = memory_access.graph() {
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
    let memory: Arc<dyn MemoryTrait> = Arc::new(memory_access);
    tracing::info!("memory system opened");

    if let Err(e) = config.personality.validate() {
        tracing::warn!("invalid personality config: {e}, using defaults");
    }

    let rules_engine = RulesEngine::new(memory.as_ref());
    match load_configured_defaults(&rules_engine, &config.consciousness.initial_rules) {
        Ok(n) if n > 0 => tracing::info!("loaded {n} default behavioral rules"),
        Ok(_) => tracing::debug!("behavioral rules already present"),
        Err(e) => tracing::warn!("failed to load default rules: {e}"),
    }

    if archon_core::update::should_auto_check(&config.update) {
        let update_config = config.update.clone();
        observability::spawn_named("auto-update-check", async move {
            match archon_core::update::check_update(&update_config).await {
                Ok(msg) => tracing::info!("auto-update check: {msg}"),
                Err(e) => tracing::debug!("auto-update check: {e}"),
            }
            archon_core::update::record_check_time();
        });
    }

    let fast_mode_shared = Arc::new(AtomicBool::new(cli.fast));
    let sandbox_flag = Arc::new(AtomicBool::new(false));
    let fast_mode = FastModeState::new_with(cli.fast);
    if cli.fast {
        tracing::info!("fast mode enabled via --fast flag");
    }

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
    } else if let Ok(level) = effort::parse_level(&config.api.default_effort) {
        effort_state.set_level(level);
        initial_effort = level;
    }
    let effort_level_shared = Arc::new(tokio::sync::Mutex::new(initial_effort));
    let model_override_shared = Arc::new(tokio::sync::Mutex::new(String::new()));

    let cost_alert_state = CostAlertState::new(&config.cost);
    tracing::debug!(
        "cost alerts: warn={}, hard={}",
        config.cost.warn_threshold,
        config.cost.hard_limit
    );

    let checkpoint_db_path = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("archon")
        .join("checkpoints.db");
    let checkpoint_store = if config.checkpoint.enabled {
        match archon_session::checkpoint::CheckpointStore::open_with_limit(
            &checkpoint_db_path,
            config.checkpoint.max_checkpoints,
        ) {
            Ok(store) => {
                tracing::info!("checkpoint store opened at {}", checkpoint_db_path.display());
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

    let working_dir = std::env::current_dir().unwrap_or_default();
    let hook_registry =
        crate::runtime::hooks::load_runtime_hook_registry(&working_dir);
    let mcp_configs = if resolved_flags.bare_mode {
        tracing::info!("bare mode: skipping MCP auto-discovery");
        Vec::new()
    } else {
        archon_mcp::config::load_merged_configs(&working_dir).unwrap_or_else(|e| {
            tracing::warn!("failed to load MCP configs: {e}");
            Vec::new()
        })
    };

    let mcp_manager = archon_mcp::lifecycle::McpServerManager::new();
    let mcp_tools = if !mcp_configs.is_empty() {
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

    let (provider_override, anthropic_client, session_api_url, prompt_identity) =
        if super::is_codex_session(config) {
            tracing::info!(
                "LLM provider selected: openai-codex (skipping Anthropic auth bootstrap)"
            );
            crate::runtime::hooks::fire_provider_resolve_hook(
                &hook_registry,
                &working_dir,
                session_id,
                crate::runtime::hooks::ProviderResolveHookPayload {
                    hook_event: "BeforeProviderResolve",
                    stage: "before_provider_resolve",
                    surface: "tui_session",
                    requested_provider: "openai-codex",
                    selected_provider: None,
                    runtime_mode: None,
                    profile_id: None,
                },
            )
            .await;
            let prompt_identity = IdentityProvider::new(
                IdentityMode::Clean,
                session_id.to_string(),
                get_or_create_device_id(),
                String::new(),
            );
            let codex_provider = super::build_codex_session_provider(config).await?;
            let selected_provider = codex_provider.name().to_string();
            let profile_id =
                crate::runtime::provider_auth_selection::selected_provider_auth_profile_id(
                    &selected_provider,
                );
            crate::runtime::hooks::fire_provider_resolve_hook(
                &hook_registry,
                &working_dir,
                session_id,
                crate::runtime::hooks::ProviderResolveHookPayload {
                    hook_event: "AfterProviderResolve",
                    stage: "after_provider_resolve",
                    surface: "tui_session",
                    requested_provider: "openai-codex",
                    selected_provider: Some(&selected_provider),
                    runtime_mode: Some(
                        crate::runtime::provider_observer::runtime_mode_for_provider_name(
                            &selected_provider,
                        ),
                    ),
                    profile_id: profile_id.as_deref(),
                },
            )
            .await;
            (Some(codex_provider), None, None, prompt_identity)
        } else if config.llm.provider != "anthropic" {
            let prompt_identity = IdentityProvider::new(
                IdentityMode::Clean,
                session_id.to_string(),
                get_or_create_device_id(),
                String::new(),
            );
            (None, None, None, prompt_identity)
        } else {
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
                    archon_llm::auth::AuthProvider::CodexOAuthToken(_) => {
                        tracing::info!("authenticated via Codex OAuth token");
                        a
                    }
                },
                Err(e) => {
                    eprintln!("Authentication failed: {e}");
                    eprintln!("Run `archon login` or set ANTHROPIC_API_KEY.");
                    std::process::exit(1);
                }
            };

            let device_id = get_or_create_device_id();
            let identity_mode =
                resolve_identity_mode(&auth, cli.identity_spoof, &config.identity.as_view());
            tracing::debug!(
                "Identity mode: {}",
                match &identity_mode {
                    IdentityMode::Clean => "clean",
                    IdentityMode::Spoof { .. } => "spoof",
                    IdentityMode::Custom { .. } => "custom",
                }
            );

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
            let session_api_url = api_url.clone();
            let api_client = AnthropicClient::new(auth, identity.clone(), api_url);

            if matches!(identity.mode, IdentityMode::Spoof { .. })
                && config.identity.spoof_betas.is_none()
            {
                let client_for_discovery = api_client.clone();
                observability::spawn_named("beta-discovery", async move {
                    let validated = resolve_and_validate_betas(&client_for_discovery, None).await;
                    tracing::info!(
                        "Background beta discovery complete: {} betas validated",
                        validated.len()
                    );
                });
            }

            (None, Some(api_client), session_api_url, identity)
        };

    Ok(Bootstrap {
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
    })
}
