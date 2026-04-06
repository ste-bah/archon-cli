mod cli_args;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use clap::Parser;

use archon_consciousness::assembler::{AssemblyInput, BudgetConfig, SystemPromptAssembler};
use archon_consciousness::defaults::load_configured_defaults;
use archon_consciousness::rules::RulesEngine;
use archon_core::agent::{Agent, AgentConfig, AgentEvent, SessionStats};
use archon_core::cli_flags::resolve_flags;
use archon_core::config::LlmConfig;
use archon_core::config::default_config_path;
use archon_core::config_layers::ConfigLayer;
use archon_core::cost_alerts::{CostAlertAction, CostAlertState};
use archon_core::dispatch::create_default_registry;
use archon_core::env_vars::{self, ArchonEnvVars};
use archon_core::input_format::InputFormat;
use archon_core::logging::{default_log_dir, init_logging, rotate_logs};
use archon_core::output_format::OutputFormat;
use archon_core::print_mode::{PrintModeConfig, run_print_mode};
use archon_core::reasoning::build_environment_section;
use archon_core::skills::builtin::register_builtins;
use archon_core::skills::discovery::discover_user_skills;
use archon_core::skills::{SkillContext, SkillOutput};
use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::resolve_auth_with_keys;
use archon_llm::effort::{self, EffortLevel, EffortState};
use archon_llm::fast_mode::FastModeState;
use archon_llm::identity::{
    IdentityMode, IdentityProvider, get_or_create_device_id, resolve_and_validate_betas,
    resolve_betas,
};
use archon_llm::provider::LlmProvider;
use archon_mcp::lifecycle::McpServerManager;
use archon_memory::{MemoryAccess, MemoryGraph, MemoryTrait};
use archon_permissions::auto::{AutoModeConfig, AutoModeEvaluator};
use archon_tui::app::{TuiEvent, run_tui};

use cli_args::{Cli, Commands};

/// Parse `--setting-sources` names into [`ConfigLayer`] variants, warning on
/// unrecognised values.
fn parse_layer_filter(sources: &[String]) -> Vec<ConfigLayer> {
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

/// Strip `cache_control` keys from system prompt blocks when prompt caching
/// is disabled via `config.context.prompt_cache = false` (TASK-WIRE-003).
/// A no-op when `prompt_cache_enabled` is true.
fn strip_cache_control_if_disabled(blocks: &mut [serde_json::Value], prompt_cache_enabled: bool) {
    if prompt_cache_enabled {
        return;
    }
    for block in blocks.iter_mut() {
        if let Some(obj) = block.as_object_mut() {
            obj.remove("cache_control");
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load environment variables first (determines config path)
    let env_vars = env_vars::load_env_vars();

    // Warn about unrecognized ARCHON_* vars
    let all_env: std::collections::HashMap<String, String> = std::env::vars().collect();
    let unrecognized = env_vars::warn_unrecognized_archon_vars(&all_env);
    for var_name in &unrecognized {
        eprintln!("warning: unrecognized environment variable: {var_name}");
    }

    // Load config from env-specified dir or default
    let config_path = env_vars
        .config_dir
        .as_ref()
        .map(|d| d.join("config.toml"))
        .unwrap_or_else(default_config_path);

    // Parse --setting-sources filter
    let layer_filter: Option<Vec<ConfigLayer>> =
        cli.setting_sources.as_ref().map(|s| parse_layer_filter(s));

    let working_dir_for_config = std::env::current_dir().unwrap_or_default();
    let mut config = archon_core::config_layers::load_layered_config(
        Some(&config_path),
        &working_dir_for_config,
        cli.settings.as_deref(),
        layer_filter.as_deref(),
    )
    .unwrap_or_else(|e| {
        eprintln!("warning: failed to load config, using defaults: {e}");
        archon_core::config::ArchonConfig::default()
    });

    // Apply env var overrides on top of config file
    env_vars::apply_env_overrides(&mut config, &env_vars);

    // ── Resolve expanded CLI flags (CLI-220) ───────────────────
    let resolved_flags = resolve_flags(&cli.to_flag_input()).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });

    // --model overrides config default model (higher priority than env var)
    if let Some(ref model) = resolved_flags.model {
        config.api.default_model = model.clone();
    }

    // --verbose bumps logging to trace
    if resolved_flags.verbose {
        config.logging.level = "trace".to_string();
    }

    // --debug sets debug-level logging with optional category filter
    if let Some(ref filter) = resolved_flags.debug {
        match filter {
            Some(categories) => {
                // e.g. "mcp,agent" -> set those specific targets to debug
                config.logging.level = format!(
                    "warn,{}",
                    categories
                        .split(',')
                        .map(|c| format!("{c}=debug"))
                        .collect::<Vec<_>>()
                        .join(",")
                );
            }
            None => {
                config.logging.level = "debug".to_string();
            }
        }
    }

    // ARCHON_LOG env var overrides log level (e.g. ARCHON_LOG=debug)
    if let Ok(log_level) = std::env::var("ARCHON_LOG")
        && !log_level.trim().is_empty()
    {
        config.logging.level = log_level.trim().to_string();
    }

    // Initialize logging
    let session_id = uuid::Uuid::new_v4().to_string();
    // ARCHON_LOG_DIR env override lets tests and hermetic environments redirect
    // log output to a known directory (dirs::data_dir() is platform-specific
    // and does NOT honor XDG_DATA_HOME on macOS/Windows).
    let log_dir = std::env::var_os("ARCHON_LOG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_log_dir);
    let _log_guard =
        init_logging(&session_id, &config.logging.level, &log_dir).unwrap_or_else(|e| {
            eprintln!("fatal: logging init failed: {e}");
            std::process::exit(1);
        });

    if let Err(e) = rotate_logs(&log_dir, config.logging.max_files) {
        tracing::warn!("failed to rotate logs: {e}");
    }
    tracing::debug!(
        "logging: max_files={}, max_file_size_mb={}",
        config.logging.max_files,
        config.logging.max_file_size_mb,
    );

    tracing::info!("Archon CLI v0.1.0 started, session {session_id}");
    if config.memory.enabled {
        tracing::info!("memory.enabled=true: memory tools + graph injection ACTIVE");
    } else {
        tracing::info!("memory.enabled=false: memory tools and graph injection DISABLED");
    }
    if config.context.prompt_cache {
        tracing::info!("context.prompt_cache=true: cache_control hints ACTIVE");
    } else {
        tracing::info!("context.prompt_cache=false: cache_control hints DISABLED");
    }
    tracing::debug!(
        "context: compact_threshold={}, max_tokens={:?}",
        config.context.compact_threshold,
        config.context.max_tokens,
    );
    let mut voice_event_rx: Option<tokio::sync::mpsc::Receiver<archon_tui::app::TuiEvent>> = None;
    if config.voice.enabled {
        use archon_tui::app::TuiEvent as VTuiEvent;
        use archon_tui::voice::pipeline::{
            AudioSource, MockAudioSource, VoicePipeline, VoiceTrigger, hotkey_action_for_mode,
            install_toggle_mode, install_trigger_sender, voice_loop,
        };
        use archon_tui::voice::stt::{LocalStt, MockStt, OpenAiStt, SttProvider};
        use std::sync::Arc as StdArc;

        let (trig_tx, trig_rx) = tokio::sync::mpsc::channel::<VoiceTrigger>(16);
        install_trigger_sender(trig_tx);
        install_toggle_mode(config.voice.toggle_mode);
        tracing::info!(
            "voice: toggle_mode={} (hotkey action={:?})",
            config.voice.toggle_mode,
            hotkey_action_for_mode(config.voice.toggle_mode)
        );
        let (voice_evt_tx, voice_evt_rx_inner) = tokio::sync::mpsc::channel::<VTuiEvent>(16);
        voice_event_rx = Some(voice_evt_rx_inner);
        let audio_capture = archon_tui::voice::capture::AudioCapture::new();
        let audio: StdArc<dyn AudioSource> = if audio_capture.is_supported() {
            tracing::info!(
                "voice: real audio device detected (sample_rate={}, channels={})",
                audio_capture.sample_rate,
                audio_capture.channels
            );
            // TODO: Wire CpalAudioSource when AudioCapture implements AudioSource
            StdArc::new(MockAudioSource::with_samples(vec![
                0.0_f32;
                audio_capture.sample_rate
                    as usize
            ]))
        } else {
            tracing::warn!("voice: no audio device available, using mock audio source");
            StdArc::new(MockAudioSource::with_samples(vec![0.0_f32; 16000]))
        };
        let stt: StdArc<dyn SttProvider> = match config.voice.stt_provider.as_str() {
            "openai" if !config.voice.stt_api_key.is_empty() => StdArc::new(OpenAiStt {
                api_key: config.voice.stt_api_key.clone(),
                url: config.voice.stt_url.clone(),
            }),
            "local" => StdArc::new(LocalStt {
                url: config.voice.stt_url.clone(),
            }),
            _ => StdArc::new(MockStt {
                response: "[voice: no STT configured]".to_string(),
            }),
        };
        let pipeline = VoicePipeline::new(audio, stt, config.voice.vad_threshold);
        tokio::spawn(async move {
            voice_loop(trig_rx, voice_evt_tx, pipeline).await;
        });
        tracing::info!(
            "voice: pipeline wired (provider={}, device={}, hotkey={})",
            config.voice.stt_provider,
            config.voice.device,
            config.voice.hotkey,
        );
        // Give the spawned voice_loop task a chance to emit its startup log.
        tokio::task::yield_now().await;
    } else {
        tracing::info!("voice: disabled (config.voice.enabled=false)");
    }

    // Handle subcommands
    match cli.command {
        Some(Commands::Login) => {
            return handle_login(&config).await;
        }
        Some(Commands::Plugin { action }) => {
            return handle_plugin_command(action);
        }
        Some(Commands::Update { check, force }) => {
            if check {
                match archon_core::update::check_update(&config.update).await {
                    Ok(msg) => println!("{msg}"),
                    Err(e) => eprintln!("update check failed: {e}"),
                }
            } else {
                match archon_core::update::perform_update(&config.update, force).await {
                    Ok(msg) => println!("{msg}"),
                    Err(archon_core::update::UpdateError::UpToDate(msg)) => println!("{msg}"),
                    Err(e) => eprintln!("update failed: {e}"),
                }
            }
            return Ok(());
        }
        Some(Commands::Remote { action }) => {
            use cli_args::RemoteAction;
            match action {
                RemoteAction::Ssh {
                    target,
                    command,
                    port,
                    key,
                } => {
                    use archon_core::remote::{
                        RemoteTransport, SshConnectionConfig, SyncMode, protocol::AgentMessage,
                        ssh::SshTransport,
                    };
                    let (user, host) = target
                        .split_once('@')
                        .map(|(u, h)| (u.to_string(), h.to_string()))
                        .unwrap_or_else(|| ("root".to_string(), target.clone()));
                    let remote_session_id = cli
                        .session_id
                        .clone()
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    tracing::info!(
                        "remote ssh: user={user} host={host} port={port} session_id={remote_session_id}"
                    );
                    println!(
                        "Remote SSH: connecting to {user}@{host}:{port} (session {remote_session_id})"
                    );
                    tracing::info!(
                        "remote ssh: agent_forwarding={} (from config.remote.ssh.agent_forwarding)",
                        config.remote.ssh.agent_forwarding
                    );
                    let ssh_cfg = SshConnectionConfig {
                        host: host.clone(),
                        port,
                        user: user.clone(),
                        key_file: key.clone(),
                        agent_forwarding: config.remote.ssh.agent_forwarding,
                        session_id: remote_session_id.clone(),
                        sync_mode: match config.remote.sync_mode.as_str() {
                            "auto" => SyncMode::Auto,
                            _ => SyncMode::Manual,
                        },
                    };
                    let session = match SshTransport.connect(&ssh_cfg).await {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("SSH connection failed: {e}");
                            std::process::exit(1);
                        }
                    };
                    println!("Connected. Session: {}", session.session_id);
                    if let Some(cmd) = command {
                        let msg = AgentMessage::UserMessage { content: cmd };
                        if let Err(e) = session.send(&msg).await {
                            eprintln!("SSH send failed: {e}");
                            let _ = session.disconnect().await;
                            std::process::exit(1);
                        }
                        match session.recv().await {
                            Ok(AgentMessage::AssistantMessage { content }) => println!("{content}"),
                            Ok(AgentMessage::Error { message }) => {
                                eprintln!("remote error: {message}");
                                let _ = tokio::time::timeout(
                                    std::time::Duration::from_secs(2),
                                    async { session.disconnect().await },
                                )
                                .await;
                                std::process::exit(1);
                            }
                            Ok(other) => println!("{other:?}"),
                            Err(e) => {
                                eprintln!("SSH recv failed: {e}");
                                std::process::exit(1);
                            }
                        }
                    } else if let Err(e) = session.disconnect().await {
                        eprintln!("SSH disconnect failed: {e}");
                        std::process::exit(1);
                    }
                }
                RemoteAction::Ws { url, token } => {
                    use archon_core::remote::websocket::{WsConnectionConfig, WsTransport};
                    let remote_session_id = cli
                        .session_id
                        .clone()
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    let cfg = WsConnectionConfig {
                        url: url.clone(),
                        token: token.unwrap_or_default(),
                        reconnect: false,
                        max_reconnect_attempts: 0,
                        session_id: remote_session_id.clone(),
                    };
                    tracing::info!("remote ws: connecting to {url} session_id={remote_session_id}");
                    println!("Remote WebSocket: connecting to {url} (session {remote_session_id})");
                    match WsTransport.connect_ws(&cfg).await {
                        Ok(session) => println!("Connected. Session: {}", session.session_id),
                        Err(e) => {
                            eprintln!("WebSocket connection failed: {e}");
                            std::process::exit(1);
                        }
                    }
                }
            }
            return Ok(());
        }
        Some(Commands::Serve { port, token_path }) => {
            use archon_core::remote::{
                server::WebSocketServer,
                websocket::{IdeHandlerFn, WsServerConfig},
            };
            use archon_sdk::ide::handler::IdeProtocolHandler;
            use std::sync::Arc;
            use tokio::sync::Mutex;
            let mut srv_cfg = WsServerConfig::default();
            // CLI --port overrides config; config overrides default
            srv_cfg.port = port;
            // Wire TLS from config (CLI has no TLS flags; config.toml is the source)
            srv_cfg.tls_cert = config.ws_remote.tls_cert.as_ref().map(PathBuf::from);
            srv_cfg.tls_key = config.ws_remote.tls_key.as_ref().map(PathBuf::from);
            // Wire --token-path: load or create token from the specified file
            if let Some(ref tp) = token_path {
                if let Ok(tok) = std::fs::read_to_string(tp) {
                    srv_cfg.token = Some(tok.trim().to_string());
                }
            }
            // Wire the real IdeProtocolHandler — archon-core cannot depend on archon-sdk,
            // so we inject it here via a boxed FnMut closure.
            let ide_proto = IdeProtocolHandler::new(env!("CARGO_PKG_VERSION"));
            let ide_handler: IdeHandlerFn = Arc::new(Mutex::new(Box::new({
                let mut h = ide_proto;
                move |req: &str| h.handle(req)
            })));
            srv_cfg.ide_handler = Some(ide_handler);
            match WebSocketServer::new(srv_cfg).await {
                Ok(server) => {
                    if let Err(e) = server.run().await {
                        eprintln!("server error: {e}");
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("failed to start server: {e}");
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        Some(Commands::Team { action }) => {
            use archon_core::orchestrator::{Orchestrator, RealSubtaskExecutor};
            use cli_args::TeamAction;
            use std::sync::Arc;
            match action {
                TeamAction::Run { team, goal } => {
                    let orch = Orchestrator::new(config.orchestrator.clone());
                    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
                    // Build LLM provider for team execution
                    let team_auth = match archon_llm::auth::resolve_auth_with_keys(
                        env_vars.anthropic_api_key.as_deref(),
                        env_vars.archon_api_key.as_deref(),
                        env_vars.archon_oauth_token.as_deref(),
                        std::env::var("ANTHROPIC_AUTH_TOKEN").ok().as_deref(),
                    ) {
                        Ok(a) => a,
                        Err(e) => {
                            eprintln!("Authentication failed for team execution: {e}");
                            eprintln!("Run `archon login` or set ANTHROPIC_API_KEY.");
                            std::process::exit(1);
                        }
                    };
                    let team_identity = archon_llm::identity::IdentityProvider::new(
                        archon_llm::identity::IdentityMode::Clean,
                        uuid::Uuid::new_v4().to_string(),
                        "team-device".to_string(),
                        String::new(),
                    );
                    let team_api_url = std::env::var("ANTHROPIC_BASE_URL")
                        .ok()
                        .or_else(|| config.api.base_url.clone());
                    let team_client = archon_llm::anthropic::AnthropicClient::new(
                        team_auth,
                        team_identity,
                        team_api_url,
                    );
                    let team_provider = build_llm_provider(&config.llm, team_client);
                    let cwd = std::env::current_dir().unwrap_or_default();
                    let executor = Arc::new(RealSubtaskExecutor::new(
                        team_provider,
                        cwd,
                        config.api.default_model.clone(),
                    ));
                    let team_cfg = archon_core::orchestrator::config::TeamConfig {
                        name: team.clone(),
                        ..Default::default()
                    };
                    tokio::spawn(async move {
                        while let Some(event) = rx.recv().await {
                            use archon_core::orchestrator::events::OrchestratorEvent;
                            match event {
                                OrchestratorEvent::TaskDecomposed { subtasks } => {
                                    println!("  Plan: {} subtasks", subtasks.len());
                                }
                                OrchestratorEvent::AgentSpawned {
                                    agent_type,
                                    subtask_id,
                                    ..
                                } => {
                                    println!("  [spawn] {agent_type} → subtask {subtask_id}");
                                }
                                OrchestratorEvent::AgentComplete { subtask_id, .. } => {
                                    println!("  [done] subtask {subtask_id}");
                                }
                                OrchestratorEvent::TeamComplete { result } => {
                                    println!("Team complete:\n{result}");
                                }
                                _ => {}
                            }
                        }
                    });
                    match orch.run_team(team_cfg, goal, executor, tx).await {
                        Ok(result) => println!("Result: {result}"),
                        Err(e) => {
                            eprintln!("Team run failed: {e}");
                            std::process::exit(1);
                        }
                    }
                }
                TeamAction::List => {
                    use archon_core::team::TeamManager;
                    let cwd =
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                    let manager = TeamManager::new(cwd.clone());
                    match manager.list_teams() {
                        Ok(ids) if ids.is_empty() => {
                            println!("No teams found in {}/teams", cwd.display());
                        }
                        Ok(ids) => {
                            println!("Teams ({}):", ids.len());
                            for id in ids {
                                match manager.load_team(&id) {
                                    Ok(cfg) => println!(
                                        "  {id:<24}  {name}  ({n} members)",
                                        name = cfg.name,
                                        n = cfg.members.len()
                                    ),
                                    Err(e) => println!("  {id:<24}  <unreadable team.json: {e}>"),
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to list teams: {e}");
                            std::process::exit(1);
                        }
                    }
                }
            }
            return Ok(());
        }
        Some(Commands::IdeStdio) => {
            use archon_sdk::ide::handler::IdeProtocolHandler;
            use archon_sdk::ide::stdio::StdioTransport;

            let handler = IdeProtocolHandler::new(env!("CARGO_PKG_VERSION"));
            let mut transport = StdioTransport::new(handler);
            // In IDE mode, the agent event channel will be wired by the IDE
            // handler's prompt method once sessions are fully connected (Phase 6).
            // For now, create a placeholder channel so the transport compiles.
            let (_event_tx, event_rx) =
                tokio::sync::mpsc::channel::<archon_core::agent::AgentEvent>(256);
            let session_id = uuid::Uuid::new_v4().to_string();
            tracing::info!("IDE stdio mode: session={session_id}");
            if let Err(e) = transport.run_with_events(event_rx, &session_id).await {
                tracing::error!("IDE stdio error: {e}");
                return Err(e);
            }
            return Ok(());
        }
        Some(Commands::Web {
            port,
            bind_address,
            no_open,
        }) => {
            use archon_sdk::web::{WebConfig, WebServer};

            // CLI args override config-file values; config.web provides defaults.
            let effective_port = port.unwrap_or(config.web.port);
            let effective_bind = bind_address.unwrap_or_else(|| config.web.bind_address.clone());
            let effective_open = if no_open {
                false
            } else {
                config.web.open_browser
            };

            // Bearer token: required for non-localhost to prevent unauthenticated access.
            let is_local = matches!(effective_bind.as_str(), "127.0.0.1" | "::1" | "localhost");
            let token = if is_local {
                None
            } else {
                Some(
                    archon_core::remote::auth::load_or_create_token()
                        .unwrap_or_else(|_| String::new()),
                )
            };

            let web_cfg = WebConfig {
                port: effective_port,
                bind_address: effective_bind,
                open_browser: effective_open,
            };

            let server = WebServer::new(web_cfg, token);
            if let Err(e) = server.run().await {
                eprintln!("web server error: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        None => {}
    }

    // ── Headless mode (--headless) ───────────────────────────────
    if cli.headless {
        let headless_session_id = cli
            .session_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        tracing::info!("headless mode: session_id={headless_session_id}");
        archon_core::headless::HeadlessRuntime::new(headless_session_id)
            .run()
            .await?;
        return Ok(());
    }

    // ── Output style: --list-output-styles (CLI-310) ─────────────
    if cli.list_output_styles {
        use archon_core::output_style::OutputStyleRegistry;
        use archon_core::output_style_loader::load_styles_from_dir;

        let mut reg = OutputStyleRegistry::new();

        // Load user styles from ~/.claude/output-styles/
        if let Some(home) = dirs::home_dir() {
            let user_styles_dir = home.join(".claude").join("output-styles");
            for style in load_styles_from_dir(&user_styles_dir) {
                reg.register(style);
            }
        }

        println!("Available output styles:");
        for name in reg.list() {
            let style = reg.get(&name).unwrap();
            let has_prompt = if style.prompt.is_some() {
                "injects prompt"
            } else {
                "no injection"
            };
            println!("  {:20} {} [{}]", style.name, style.description, has_prompt);
        }
        return Ok(());
    }

    // ── Theme: --list-themes (CLI-315) ───────────────────────────
    if cli.list_themes {
        use archon_tui::theme::available_themes;
        use archon_tui::theme_registry::detect_system_theme;

        println!("Available themes:");
        for name in available_themes() {
            println!("  {name}");
        }
        println!("  daltonized  (colorblind-friendly)");
        println!("  auto        (system dark/light detection → {:?})", {
            let detected = detect_system_theme();
            let dark_bg = archon_tui::theme::dark_theme().bg;
            if detected.bg == dark_bg {
                "dark"
            } else {
                "light"
            }
        });

        if let Some(theme_name) = cli.theme.as_deref().or(config.tui.theme.as_deref()) {
            let resolved = archon_tui::theme_registry::ThemeRegistry::new().resolve(theme_name);
            println!(
                "\nActive theme: {theme_name}  (bg={:?}, fg={:?})",
                resolved.bg, resolved.fg
            );
        }

        return Ok(());
    }

    // Handle --resume with no ID: list recent sessions and exit
    if let Some(None) = &cli.resume {
        return handle_resume_list().await;
    }

    // For --resume with ID, load the session messages to restore
    let mut resume_messages = if let Some(Some(ref id)) = cli.resume {
        Some(load_resume_messages(id)?)
    } else {
        None
    };

    // ── --continue: resume most recent session in this directory ──
    if cli.continue_session && resume_messages.is_none() {
        let cwd = std::env::current_dir().unwrap_or_default();
        let cwd_str = cwd.to_string_lossy().to_string();
        let db_path = archon_session::storage::default_db_path();
        if let Ok(store) = archon_session::storage::SessionStore::open(&db_path) {
            match archon_session::listing::most_recent_in_directory(&store, &cwd_str) {
                Ok(Some(meta)) => {
                    eprintln!(
                        "Continuing session {} ...",
                        &meta.id[..8.min(meta.id.len())],
                    );
                    match archon_session::resume::resume_session(&store, &meta.id) {
                        Ok((_m, raw_messages)) => {
                            let messages: Vec<serde_json::Value> = raw_messages
                                .iter()
                                .filter_map(|s| serde_json::from_str(s).ok())
                                .collect();
                            resume_messages = Some(messages);
                        }
                        Err(e) => {
                            eprintln!("Failed to continue session: {e}");
                        }
                    }
                }
                Ok(None) => {
                    eprintln!("No previous session found in this directory.");
                }
                Err(e) => {
                    eprintln!("Session lookup failed: {e}");
                }
            }
        }
    }

    // ── Auto-resume (TASK-WIRE-004) ────────────────────────────
    // Priority: explicit --resume > --continue > --no-resume > config.session.auto_resume.
    if cli.resume.is_some() || cli.continue_session {
        tracing::info!("auto_resume: skipped (--resume specified)");
    } else if cli.no_resume {
        tracing::info!("auto_resume: skipped (--no-resume)");
    } else if !config.session.auto_resume {
        tracing::info!("auto_resume: skipped (session.auto_resume=false)");
    } else {
        // auto_resume is enabled. Look up the most-recent session for this cwd.
        let cwd = std::env::current_dir().unwrap_or_default();
        let cwd_str = cwd.to_string_lossy().to_string();
        let db_path = archon_session::storage::default_db_path();
        match archon_session::storage::SessionStore::open(&db_path) {
            Ok(store) => {
                match archon_session::listing::most_recent_in_directory(&store, &cwd_str) {
                    Ok(Some(meta)) => {
                        tracing::info!(
                            "auto_resume: found prior session {} ({} messages) for {}",
                            &meta.id[..8.min(meta.id.len())],
                            meta.message_count,
                            cwd_str
                        );
                        eprintln!(
                            "Auto-resumed session {} — pass --no-resume to start fresh.",
                            &meta.id[..8.min(meta.id.len())],
                        );
                        match archon_session::resume::resume_session(&store, &meta.id) {
                            Ok((_m, raw_messages)) => {
                                let messages: Vec<serde_json::Value> = raw_messages
                                    .iter()
                                    .filter_map(|s| serde_json::from_str(s).ok())
                                    .collect();
                                resume_messages = Some(messages);
                            }
                            Err(e) => {
                                tracing::warn!("auto_resume: failed to load messages: {e}");
                            }
                        }
                    }
                    Ok(None) => {
                        tracing::info!("auto_resume: no prior session for this directory");
                    }
                    Err(e) => {
                        tracing::warn!("auto_resume: lookup failed: {e}");
                    }
                }
            }
            Err(e) => {
                tracing::warn!("auto_resume: failed to open session store: {e}");
            }
        }
    }

    // ── Session search & management (CLI-208) ──────────────────
    if cli.sessions {
        return handle_sessions(&cli);
    }

    // ── Background sessions (CLI-221) ─────────────────────────
    if cli.ps {
        return handle_bg_list();
    }
    if let Some(ref id) = cli.kill_session {
        return handle_bg_kill(id);
    }
    if let Some(ref id) = cli.attach {
        return handle_bg_attach(id);
    }
    if let Some(ref id) = cli.logs {
        return handle_bg_logs(id);
    }
    if cli.bg.is_some() {
        return handle_bg_launch(&cli);
    }

    // ── Print mode: non-interactive single-query ──────────────
    if cli.print.is_some() {
        let query = match &cli.print {
            Some(Some(q)) => q.clone(),
            Some(None) => {
                // Read from stdin based on input format
                let input_fmt = InputFormat::from_str(&cli.input_format).unwrap_or_else(|e| {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                });
                let messages =
                    archon_core::input_format::read_input(&input_fmt).unwrap_or_else(|e| {
                        eprintln!("error reading input: {e}");
                        std::process::exit(1);
                    });
                messages.join("\n")
            }
            None => unreachable!(),
        };

        let output_fmt = OutputFormat::from_str(&cli.output_format).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1);
        });

        let print_config = PrintModeConfig {
            query,
            output_format: output_fmt,
            input_format: InputFormat::from_str(&cli.input_format).unwrap_or(InputFormat::Text),
            max_turns: cli.max_turns,
            max_budget_usd: cli.max_budget_usd,
            no_session_persistence: cli.no_session_persistence,
            json_schema: cli.json_schema.clone(),
        };

        // Build a minimal agent for print mode (no TUI)
        let exit_code = run_print_mode_session(
            &config,
            &session_id,
            &cli,
            &env_vars,
            print_config,
            &resolved_flags,
        )
        .await;
        std::process::exit(exit_code);
    }

    // Default: interactive session (with optional resume messages)
    run_interactive_session(
        &config,
        &session_id,
        &cli,
        &env_vars,
        resume_messages,
        &resolved_flags,
        voice_event_rx,
    )
    .await
}

fn handle_plugin_command(action: cli_args::PluginAction) -> Result<()> {
    use archon_plugin::loader::PluginLoader;

    let plugins_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("archon")
        .join("plugins");

    // Check ARCHON_PLUGIN_SEED_DIR env var
    let seed_dirs: Vec<std::path::PathBuf> = std::env::var("ARCHON_PLUGIN_SEED_DIR")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .collect();

    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from(".cache"))
        .join("archon")
        .join("wasm");
    let mut loader =
        PluginLoader::new(plugins_dir).with_cache(archon_plugin::cache::WasmCache::new(cache_dir));
    if !seed_dirs.is_empty() {
        loader = loader.with_seed_dirs(seed_dirs);
    }
    let result = loader.load_all();

    match action {
        cli_args::PluginAction::List => {
            println!("{:<30} {:<12} STATUS", "NAME", "VERSION");
            println!("{}", "-".repeat(56));
            for plugin in &result.enabled {
                println!(
                    "{:<30} {:<12} enabled",
                    plugin.manifest.name, plugin.manifest.version
                );
            }
            for plugin in &result.disabled {
                println!(
                    "{:<30} {:<12} disabled",
                    plugin.manifest.name, plugin.manifest.version
                );
            }
            for (id, err) in &result.errors {
                println!("{:<30} {:<12} error: {err}", id, "?");
            }
            if result.enabled.is_empty() && result.disabled.is_empty() && result.errors.is_empty() {
                println!("No plugins found.");
            }
        }
        cli_args::PluginAction::Info { name } => {
            let plugin = result
                .enabled
                .iter()
                .chain(result.disabled.iter())
                .find(|p| p.manifest.name == name);
            match plugin {
                Some(p) => {
                    let status = if result.disabled.iter().any(|d| d.manifest.name == name) {
                        "disabled"
                    } else {
                        "enabled"
                    };
                    println!("Name:        {}", p.manifest.name);
                    println!("Version:     {}", p.manifest.version);
                    println!("Status:      {status}");
                    if let Some(desc) = &p.manifest.description {
                        println!("Description: {desc}");
                    }
                    if !p.manifest.capabilities.is_empty() {
                        println!("Capabilities: {}", p.manifest.capabilities.join(", "));
                    }
                    println!("Data dir:    {}", p.data_dir.display());
                }
                None => {
                    // Check errors
                    if let Some((_, err)) = result.errors.iter().find(|(id, _)| id == &name) {
                        eprintln!("Plugin '{name}' failed to load: {err}");
                    } else {
                        eprintln!("Plugin '{name}' not found.");
                    }
                }
            }
        }
    }
    Ok(())
}

async fn handle_login(_config: &archon_core::config::ArchonConfig) -> Result<()> {
    let http_client = reqwest::Client::new();
    let cred_path = archon_llm::tokens::credentials_path();

    eprintln!("Starting OAuth login...");
    match archon_llm::oauth::login(&cred_path, &http_client).await {
        Ok(_) => {
            eprintln!("Login successful! Credentials saved.");
            Ok(())
        }
        Err(e) => {
            eprintln!("Login failed: {e}");
            std::process::exit(1);
        }
    }
}

// ── Background session handlers (CLI-221) ────────────────────────────────

/// List background sessions and exit.
fn handle_bg_list() -> Result<()> {
    // Clean stale PIDs first
    let _ = archon_session::registry::cleanup_stale_pids();

    let sessions = archon_session::registry::list_sessions()
        .map_err(|e| anyhow::anyhow!("failed to list background sessions: {e}"))?;

    if sessions.is_empty() {
        eprintln!("No background sessions found.");
    } else {
        eprintln!(
            "{:<10} {:<14} {:<20} {:<8} STARTED",
            "ID", "STATUS", "NAME", "TURNS"
        );
        for s in &sessions {
            let short_id = if s.id.len() > 8 { &s.id[..8] } else { &s.id };
            eprintln!(
                "{:<10} {:<14} {:<20} {:<8} {}",
                short_id, s.status, s.name, s.turns, s.started_at,
            );
        }
    }
    Ok(())
}

/// Kill a background session and exit.
#[cfg(unix)]
fn handle_bg_kill(id: &str) -> Result<()> {
    archon_session::background::kill_session(id)
        .map_err(|e| anyhow::anyhow!("failed to kill session {id}: {e}"))?;
    eprintln!("Session {id} killed.");
    Ok(())
}

#[cfg(not(unix))]
fn handle_bg_kill(id: &str) -> Result<()> {
    eprintln!("Background sessions are only supported on Unix systems.");
    std::process::exit(1);
}

/// Attach to a running background session (stream logs).
#[cfg(unix)]
fn handle_bg_attach(id: &str) -> Result<()> {
    archon_session::attach::stream_logs(id, true)
        .map_err(|e| anyhow::anyhow!("failed to attach to session {id}: {e}"))?;
    Ok(())
}

#[cfg(not(unix))]
fn handle_bg_attach(id: &str) -> Result<()> {
    eprintln!("Background sessions are only supported on Unix systems.");
    std::process::exit(1);
}

/// View background session logs (non-streaming).
fn handle_bg_logs(id: &str) -> Result<()> {
    let content = archon_session::attach::view_logs(id)
        .map_err(|e| anyhow::anyhow!("failed to read logs for session {id}: {e}"))?;
    print!("{content}");
    Ok(())
}

/// Launch a background session and exit.
#[cfg(unix)]
fn handle_bg_launch(cli: &Cli) -> Result<()> {
    let query = match &cli.bg {
        Some(Some(q)) => q.clone(),
        Some(None) => {
            // Read from stdin
            use std::io::Read as _;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| anyhow::anyhow!("failed to read stdin: {e}"))?;
            buf
        }
        None => unreachable!(),
    };

    if query.trim().is_empty() {
        eprintln!("error: no query provided for background session");
        std::process::exit(1);
    }

    let archon_binary = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("failed to resolve archon binary path: {e}"))?;

    let session_id = archon_session::background::launch_background(
        &query,
        cli.bg_name.as_deref(),
        &archon_binary,
    )
    .map_err(|e| anyhow::anyhow!("failed to launch background session: {e}"))?;

    let short_id = if session_id.len() > 8 {
        &session_id[..8]
    } else {
        &session_id
    };
    eprintln!("Background session started: {session_id}");
    eprintln!("  Attach: archon --attach {short_id}");
    eprintln!("  Logs:   archon --logs {short_id}");
    eprintln!("  Kill:   archon --kill {short_id}");
    eprintln!("  List:   archon --ps");
    Ok(())
}

#[cfg(not(unix))]
fn handle_bg_launch(_cli: &Cli) -> Result<()> {
    eprintln!("Background sessions are only supported on Unix systems.");
    std::process::exit(1);
}

/// Handle `--sessions` flag: search, stats, or delete sessions.
fn handle_sessions(cli: &Cli) -> Result<()> {
    let db_path = archon_session::storage::default_db_path();
    let store = archon_session::storage::SessionStore::open(&db_path)
        .map_err(|e| anyhow::anyhow!("failed to open session database: {e}"))?;

    // --sessions --delete <ID>
    if let Some(ref id) = cli.delete {
        store
            .delete_session(id)
            .map_err(|e| anyhow::anyhow!("failed to delete session: {e}"))?;
        eprintln!("Deleted session {id}");
        return Ok(());
    }

    // --sessions --stats
    if cli.stats {
        let stats = archon_session::search::session_stats(&store)
            .map_err(|e| anyhow::anyhow!("failed to compute stats: {e}"))?;
        println!("Sessions:  {}", stats.total_sessions);
        println!("Tokens:    {}", stats.total_tokens);
        println!("Messages:  {}", stats.total_messages);
        println!("Avg dur:   {:.0}s", stats.avg_duration_secs);
        return Ok(());
    }

    // Build search query from CLI flags.
    let after = cli.after.as_ref().map(|s| parse_datetime(s)).transpose()?;
    let before = cli.before.as_ref().map(|s| parse_datetime(s)).transpose()?;

    let query = archon_session::search::SessionSearchQuery {
        branch: cli.branch.clone(),
        directory: cli.session_dir.clone(),
        after,
        before,
        text: cli.search.clone(),
        tag: None,
        ..Default::default()
    };

    let results = archon_session::search::search_sessions(&store, &query)
        .map_err(|e| anyhow::anyhow!("search failed: {e}"))?;

    if results.is_empty() {
        eprintln!("No matching sessions found.");
    } else {
        for session in &results {
            println!("{}", archon_session::resume::format_session_line(session));
        }
    }

    Ok(())
}

/// Parse a date string as either RFC 3339 or YYYY-MM-DD (assumes midnight UTC).
fn parse_datetime(s: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    // Try RFC 3339 first.
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&chrono::Utc));
    }
    // Try YYYY-MM-DD.
    if let Ok(nd) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let naive = nd
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| anyhow::anyhow!("invalid date: {s}"))?;
        return Ok(naive.and_utc());
    }
    Err(anyhow::anyhow!(
        "invalid date format: {s} (expected RFC 3339 or YYYY-MM-DD)"
    ))
}

/// List recent sessions for `--resume` with no ID.
async fn handle_resume_list() -> Result<()> {
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
fn load_resume_messages(session_id: &str) -> Result<Vec<serde_json::Value>> {
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

/// Run a print-mode session: set up auth/agent, process one query, return exit code.
async fn run_print_mode_session(
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

    // Build a minimal system prompt (skip CLAUDE.md in bare mode)
    let claude_md = if resolved_flags.bare_mode {
        String::new()
    } else {
        archon_core::claudemd::load_hierarchical_claude_md_with_limit(
            &working_dir,
            config.context.claudemd_max_tokens as usize,
        )
    };
    let git_info = archon_core::git::detect_git_info(&working_dir);
    let git_branch = git_info.as_ref().map(|g| g.branch.as_str());
    let env_section = build_environment_section(&working_dir, git_branch);

    let mut identity_blocks = identity.system_prompt_blocks("", &claude_md, &env_section);
    // Gated by config.context.prompt_cache (TASK-WIRE-003) — strip cache_control
    // from identity blocks when disabled so print mode honours the flag too.
    strip_cache_control_if_disabled(&mut identity_blocks, config.context.prompt_cache);
    let mut system_prompt: Vec<serde_json::Value> = identity_blocks;

    // ── Output style injection for print mode (CLI-310) ──────────
    {
        use archon_core::output_style::OutputStyleRegistry;
        use archon_core::output_style_loader::load_styles_from_dir;

        let mut reg = OutputStyleRegistry::new();
        if let Some(home) = dirs::home_dir() {
            for style in load_styles_from_dir(&home.join(".claude").join("output-styles")) {
                reg.register(style);
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

    let agent_config = AgentConfig {
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
    };

    let (agent_event_tx, agent_event_rx) = tokio::sync::mpsc::channel::<AgentEvent>(256);
    let provider = build_llm_provider(&config.llm, api_client);
    tracing::info!("LLM provider: {}", provider.name());
    let mut agent = Agent::new(provider, registry, agent_config, agent_event_tx);

    // Wire auto-mode evaluator
    let auto_eval = AutoModeEvaluator::new(AutoModeConfig {
        project_dir: Some(working_dir),
        ..Default::default()
    });
    agent.set_auto_evaluator(auto_eval);

    run_print_mode(print_config, config, &mut agent, agent_event_rx).await
}

async fn run_interactive_session(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    env_vars: &ArchonEnvVars,
    resume_messages: Option<Vec<serde_json::Value>>,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    voice_event_rx: Option<tokio::sync::mpsc::Receiver<archon_tui::app::TuiEvent>>,
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
                    user_agent: custom
                        .map(|c| c.user_agent.clone())
                        .unwrap_or_else(|| "archon-cli/0.1.0".into()),
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

    let tool_defs = registry.tool_definitions();

    // ── Phase 2: Assemble system prompt with consciousness (CLI-108) ──
    let claude_md = if resolved_flags.bare_mode {
        tracing::info!("bare mode: skipping CLAUDE.md loading");
        String::new()
    } else {
        archon_core::claudemd::load_hierarchical_claude_md_with_limit(
            &working_dir,
            config.context.claudemd_max_tokens as usize,
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
    let identity_blocks = identity.system_prompt_blocks("", &claude_md, &env_section);
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
        personality: if resolved_flags.bare_mode {
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
        project_instructions: if claude_md.is_empty() {
            None
        } else {
            Some(claude_md.clone())
        },
        environment: if env_section.is_empty() {
            None
        } else {
            Some(env_section.clone())
        },
        inner_voice: None, // Populated on subsequent turns by InnerVoice::to_prompt_block()
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
    let system_prompt: Vec<serde_json::Value> =
        if let Some(ref override_text) = resolved_flags.system_prompt_override {
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
                    for style in load_styles_from_dir(&home.join(".claude").join("output-styles")) {
                        reg.register(style);
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
    let agent_config = AgentConfig {
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
    };
    let extra_dirs_shared = Arc::clone(&agent_config.extra_dirs);

    // Create channels
    let (agent_event_tx, mut agent_event_rx) = tokio::sync::mpsc::channel::<AgentEvent>(256);
    let (tui_event_tx, tui_event_rx) = tokio::sync::mpsc::channel::<TuiEvent>(256);
    // CRIT-13: Forward voice pipeline events to TUI event channel
    if let Some(mut voice_rx) = voice_event_rx {
        let voice_fwd_tx = tui_event_tx.clone();
        tokio::spawn(async move {
            while let Some(evt) = voice_rx.recv().await {
                if voice_fwd_tx.send(evt).await.is_err() {
                    break;
                }
            }
        });
    }
    let (user_input_tx, mut user_input_rx) = tokio::sync::mpsc::channel::<String>(16);

    // Create agent
    let provider = build_llm_provider(&config.llm, api_client);
    tracing::info!("LLM provider: {}", provider.name());
    let mut agent = Agent::new(provider, registry, agent_config, agent_event_tx);

    // Wire checkpoint store into agent (CLI-116)
    if let Some(store) = checkpoint_store {
        agent.set_checkpoint_store(store);
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

    // Wire hook system — load hooks from .claude/settings.json if present
    let hook_registry_arc = {
        let settings_json_path = working_dir.join(".claude").join("settings.json");
        let hook_registry = if settings_json_path.exists() {
            match std::fs::read_to_string(&settings_json_path) {
                Ok(content) => archon_core::hooks::HookRegistry::load_from_settings_json(&content)
                    .unwrap_or_else(|e| {
                        tracing::warn!("hooks: failed to load settings.json: {e}");
                        archon_core::hooks::HookRegistry::new()
                    }),
                Err(e) => {
                    tracing::warn!("hooks: could not read settings.json: {e}");
                    archon_core::hooks::HookRegistry::new()
                }
            }
        } else {
            archon_core::hooks::HookRegistry::new()
        };
        let arc = std::sync::Arc::new(hook_registry);
        agent.set_hook_registry(Arc::clone(&arc));
        arc
    };

    // GAP 6: Wire auto-mode evaluator
    let auto_eval = AutoModeEvaluator::new(AutoModeConfig {
        project_dir: Some(working_dir.clone()),
        ..Default::default()
    });
    agent.set_auto_evaluator(auto_eval);

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
            let _ = tui_event_tx.send(TuiEvent::SessionRenamed(name)).await;
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
                                let _ = watch_tui_tx.send(TuiEvent::TextDelta(msg)).await;
                            } else if !changed_keys.is_empty() {
                                let msg =
                                    format!("\nConfig reloaded: {}\n", changed_keys.join(", "));
                                let _ = watch_tui_tx.send(TuiEvent::TextDelta(msg)).await;
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
    tokio::spawn(async move {
        while let Some(event) = agent_event_rx.recv().await {
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
                AgentEvent::ToolCallComplete { name, result, .. } => TuiEvent::ToolComplete {
                    name,
                    success: !result.is_error,
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
                            let _ = tui_tx
                                .send(TuiEvent::Error(format!("COST WARNING: {msg}")))
                                .await;
                        }
                        CostAlertAction::HardLimitPause(msg) => {
                            let _ = tui_tx
                                .send(TuiEvent::Error(format!("COST LIMIT: {msg}")))
                                .await;
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
            if tui_tx.send(tui_event).await.is_err() {
                break;
            }
        }
    });

    // ── Phase 2: Slash command state (CLI-110, CLI-118, CLI-119) ──
    let mut fast_mode = fast_mode;
    let mut effort_state = effort_state;

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
            // Common aliases for built-in skills
            reg.register_alias("?", "help");
            reg.register_alias("q", "exit");
            reg
        }),
        last_assistant_response: Arc::clone(&last_assistant_response_shared),
        system_prompt_chars,
        tool_defs_chars,
        allow_bypass_permissions: cli.allow_dangerously_skip_permissions
            || cli.dangerously_skip_permissions,
        denial_log: Arc::clone(&agent.denial_log),
    };

    // Spawn agent input processor with slash command dispatch
    let input_tui_tx = tui_event_tx.clone();
    let slash_commands_disabled = resolved_flags.disable_slash_commands;
    let session_store_for_input = Arc::clone(&session_store);
    let session_id_for_input = session_id.to_string();
    // Clone api_url for the btw_tx background task (line ~1683); the spawn below consumes it.
    let api_url_for_btw = api_url.clone();
    tokio::spawn(async move {
        // CRIT-06: Fire Setup hook once agent is fully configured
        agent
            .fire_hook(
                archon_core::hooks::HookType::Setup,
                serde_json::json!({
                    "hook_event": "Setup",
                }),
            )
            .await;

        // CRIT-06: Fire SessionStart hook at the beginning of the session
        agent
            .fire_hook(
                archon_core::hooks::HookType::SessionStart,
                serde_json::json!({
                    "hook_event": "SessionStart",
                    "reason": "new_session",
                }),
            )
            .await;

        while let Some(input) = user_input_rx.recv().await {
            // Session picker selection — load messages and restore conversation
            if let Some(session_id) = input.strip_prefix("__resume_session__ ") {
                let session_id = session_id.trim();
                let db_path = archon_session::storage::default_db_path();
                match archon_session::storage::SessionStore::open(&db_path) {
                    Ok(store) => {
                        // Restore session name badge if present
                        if let Ok(meta) = store.get_session(session_id)
                            && let Some(name) = meta.name
                        {
                            let _ = input_tui_tx.send(TuiEvent::SessionRenamed(name)).await;
                        }
                        match store.load_messages(session_id) {
                            Ok(raw_messages) => {
                                // Parse JSON strings back to Values
                                let messages: Vec<serde_json::Value> = raw_messages
                                    .iter()
                                    .filter_map(|s| serde_json::from_str(s).ok())
                                    .collect();
                                let count = messages.len();
                                agent.clear_conversation().await;

                                // Display the loaded conversation history in the output
                                let _ = input_tui_tx.send(TuiEvent::TextDelta(
                                    format!("\n━━━ Resumed session {session_id} ({count} messages) ━━━\n\n")
                                )).await;
                                for msg in &messages {
                                    let role = msg["role"].as_str().unwrap_or("unknown");
                                    // Extract text content (handles both string and array formats)
                                    let content = match &msg["content"] {
                                        serde_json::Value::String(s) => s.clone(),
                                        serde_json::Value::Array(arr) => arr
                                            .iter()
                                            .filter_map(|item| {
                                                item["text"].as_str().map(|s| s.to_string())
                                            })
                                            .collect::<Vec<_>>()
                                            .join("\n"),
                                        _ => String::new(),
                                    };
                                    if content.is_empty() {
                                        continue;
                                    }
                                    let label = match role {
                                        "user" => "> ",
                                        "assistant" => "",
                                        _ => "",
                                    };
                                    let _ = input_tui_tx
                                        .send(TuiEvent::TextDelta(format!("{label}{content}\n\n")))
                                        .await;
                                }
                                let _ = input_tui_tx
                                    .send(TuiEvent::TextDelta(
                                        "━━━ End of history — continue conversation ━━━\n\n"
                                            .to_string(),
                                    ))
                                    .await;

                                agent.restore_conversation(messages);
                            }
                            Err(e) => {
                                let _ = input_tui_tx
                                    .send(TuiEvent::Error(format!("Failed to load session: {e}")))
                                    .await;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = input_tui_tx
                            .send(TuiEvent::Error(format!("Session store error: {e}")))
                            .await;
                    }
                }
                let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete).await;
                continue;
            }

            // ── MCP manager actions from the overlay ─────────────
            if let Some(rest) = input.strip_prefix("__mcp_action__ ") {
                let parts: Vec<&str> = rest.trim().splitn(2, ' ').collect();
                if parts.len() == 2 {
                    let (server_name, action) = (parts[0], parts[1]);
                    match action {
                        "reconnect" => {
                            let _ = cmd_ctx.mcp_manager.restart_server(server_name).await;
                        }
                        "disable" => {
                            let _ = cmd_ctx.mcp_manager.disable_server(server_name).await;
                        }
                        "enable" => {
                            let _ = cmd_ctx.mcp_manager.enable_server(server_name).await;
                        }
                        _ => {}
                    }
                    // Send updated state back to TUI overlay.
                    let info = cmd_ctx.mcp_manager.get_server_info().await;
                    let mut updated: Vec<archon_tui::app::McpServerEntry> = Vec::new();
                    for (name, state, disabled) in info {
                        let state_str = if disabled {
                            "disabled"
                        } else {
                            match state {
                                archon_mcp::types::ServerState::Ready => "ready",
                                archon_mcp::types::ServerState::Starting
                                | archon_mcp::types::ServerState::Restarting => "starting",
                                archon_mcp::types::ServerState::Crashed => "crashed",
                                archon_mcp::types::ServerState::Stopped => "stopped",
                            }
                        };
                        let tools = if state_str == "ready" {
                            cmd_ctx.mcp_manager.list_tools_for(&name).await
                        } else {
                            Vec::new()
                        };
                        updated.push(archon_tui::app::McpServerEntry {
                            name: name.clone(),
                            state: state_str.to_string(),
                            tool_count: tools.len(),
                            disabled,
                            tools,
                        });
                    }
                    let _ = input_tui_tx.send(TuiEvent::UpdateMcpManager(updated)).await;
                }
                let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete).await;
                continue;
            }

            // ── Phase 2: Slash command dispatch (CLI-110) ────────
            if !slash_commands_disabled && input.starts_with('/') {
                // GAP 1: /compact needs direct access to agent.compact()
                if input.trim() == "/exit" || input.trim() == "/quit" {
                    // Fire SessionEnd hook and close the TUI
                    agent
                        .fire_hook(
                            archon_core::hooks::HookType::SessionEnd,
                            serde_json::json!({"hook_type": "session_end", "reason": "exit"}),
                        )
                        .await;
                    let _ = input_tui_tx
                        .send(TuiEvent::TextDelta("\nGoodbye.\n".into()))
                        .await;
                    let _ = input_tui_tx.send(TuiEvent::Done).await;
                    continue;
                }
                if input.trim() == "/compact" || input.trim().starts_with("/compact ") {
                    let subcommand = input.trim().strip_prefix("/compact").unwrap().trim();
                    let subcommand = if subcommand.is_empty() {
                        None
                    } else {
                        Some(subcommand)
                    };
                    let msg = agent.compact(subcommand).await;
                    let _ = input_tui_tx
                        .send(TuiEvent::TextDelta(format!("\n{msg}\n")))
                        .await;
                    let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete).await;
                    continue;
                }

                // /clear needs direct access to agent.clear_conversation()
                if input.trim() == "/clear" {
                    // Fire SessionEnd hook before clearing
                    agent
                        .fire_hook(
                            archon_core::hooks::HookType::SessionEnd,
                            serde_json::json!({"hook_type": "session_end", "reason": "clear"}),
                        )
                        .await;
                    // Clear conversation
                    agent.clear_conversation().await;
                    // Reset session stats
                    {
                        let mut stats = cmd_ctx.session_stats.lock().await;
                        *stats = archon_core::agent::SessionStats::default();
                    }
                    // Clear last assistant response buffer
                    {
                        let mut resp = cmd_ctx.last_assistant_response.lock().await;
                        resp.clear();
                    }
                    // Fire SessionStart hook after
                    agent
                        .fire_hook(
                            archon_core::hooks::HookType::SessionStart,
                            serde_json::json!({"hook_type": "session_start", "reason": "clear"}),
                        )
                        .await;
                    let _ = input_tui_tx
                        .send(TuiEvent::TextDelta(
                            "\nConversation cleared. Session reset.\n".into(),
                        ))
                        .await;
                    let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete).await;
                    continue;
                }

                // /refresh-identity — clears beta caches and re-runs discovery in background
                if input.trim() == "/refresh-identity" {
                    // Clear the validated beta cache
                    let validated_cache = dirs::config_dir()
                        .unwrap_or_default()
                        .join("archon")
                        .join("validated_betas.json");
                    let _ = std::fs::remove_file(&validated_cache);
                    // Clear the raw discovered cache
                    let raw_cache = dirs::config_dir()
                        .unwrap_or_default()
                        .join("archon")
                        .join("discovered_betas.json");
                    let _ = std::fs::remove_file(&raw_cache);

                    // Spawn background re-discovery using a temporary client
                    let (refresh_auth, refresh_identity) =
                        match (agent.auth_provider(), agent.identity_provider()) {
                            (Some(a), Some(i)) => (a.clone(), i.clone()),
                            _ => {
                                let _ = input_tui_tx
                                    .send(TuiEvent::TextDelta(
                                        "\nIdentity refresh not supported for this provider.\n"
                                            .into(),
                                    ))
                                    .await;
                                continue;
                            }
                        };
                    let refresh_api_url = api_url.clone();
                    let refresh_tui_tx = input_tui_tx.clone();
                    tokio::spawn(async move {
                        let refresh_client = archon_llm::anthropic::AnthropicClient::new(
                            refresh_auth,
                            refresh_identity,
                            refresh_api_url,
                        );
                        let validated =
                            archon_llm::identity::resolve_and_validate_betas(&refresh_client, None)
                                .await;
                        tracing::info!(
                            "Identity refresh complete: {} betas validated",
                            validated.len()
                        );
                        let _ = refresh_tui_tx
                            .send(TuiEvent::TextDelta(format!(
                                "\nIdentity refresh complete: {} betas validated and cached.\n\
                                 Restart archon to apply the updated beta headers.\n",
                                validated.len()
                            )))
                            .await;
                    });

                    let _ = input_tui_tx
                        .send(TuiEvent::TextDelta(
                            "\nIdentity cache cleared. Re-discovering beta headers in background...\n"
                                .into(),
                        ))
                        .await;
                    let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete).await;
                    continue;
                }

                // /export needs access to conversation messages
                if input.trim() == "/export" || input.trim().starts_with("/export ") {
                    let format_arg = input.trim().strip_prefix("/export").unwrap_or("").trim();
                    let format = if format_arg.is_empty() {
                        archon_session::export::ExportFormat::Markdown
                    } else {
                        match archon_session::export::ExportFormat::from_str(format_arg) {
                            Ok(f) => f,
                            Err(e) => {
                                let _ = input_tui_tx.send(TuiEvent::Error(e)).await;
                                let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete).await;
                                continue;
                            }
                        }
                    };
                    let messages = &agent.conversation_state().messages;
                    match archon_session::export::export_session(
                        messages,
                        &cmd_ctx.session_id,
                        format,
                    ) {
                        Ok(content) => {
                            let export_dir = dirs::data_dir()
                                .unwrap_or_else(|| PathBuf::from("."))
                                .join("archon")
                                .join("exports");
                            if let Err(e) = std::fs::create_dir_all(&export_dir) {
                                let _ = input_tui_tx
                                    .send(TuiEvent::Error(format!(
                                        "Failed to create export dir: {e}"
                                    )))
                                    .await;
                                let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete).await;
                                continue;
                            }
                            let filename = archon_session::export::default_export_filename(
                                &cmd_ctx.session_id,
                                format,
                            );
                            let path = export_dir.join(&filename);
                            match archon_session::export::write_export(&content, &path) {
                                Ok(()) => {
                                    let _ = input_tui_tx
                                        .send(TuiEvent::TextDelta(format!(
                                            "\nExported ({format_arg_display}) to {}\n",
                                            path.display(),
                                            format_arg_display = if format_arg.is_empty() {
                                                "markdown"
                                            } else {
                                                format_arg
                                            }
                                        )))
                                        .await;
                                }
                                Err(e) => {
                                    let _ = input_tui_tx
                                        .send(TuiEvent::Error(format!("Export failed: {e}")))
                                        .await;
                                }
                            }
                        }
                        Err(e) => {
                            let _ = input_tui_tx
                                .send(TuiEvent::Error(format!("Export failed: {e}")))
                                .await;
                        }
                    }
                    let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete).await;
                    continue;
                }

                let handled = handle_slash_command(
                    input.trim(),
                    &mut fast_mode,
                    &mut effort_state,
                    &input_tui_tx,
                    &mut cmd_ctx,
                )
                .await;
                if handled {
                    let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete).await;
                    continue;
                }

                // Fallback: check the skill registry for expanded commands
                let (cmd_name, cmd_args) =
                    match archon_core::skills::parser::parse_slash_command(input.trim()) {
                        Some((name, args)) => (name, args),
                        None => (String::new(), Vec::new()),
                    };

                if let Some(skill) = cmd_ctx.skill_registry.resolve(&cmd_name) {
                    let skill_ctx = SkillContext {
                        session_id: cmd_ctx.session_id.clone(),
                        working_dir: cmd_ctx.working_dir.clone(),
                        model: cmd_ctx.default_model.clone(),
                    };
                    let output = skill.execute(&cmd_args, &skill_ctx);
                    match output {
                        SkillOutput::Prompt(prompt) => {
                            // Equivalent to Claude Code's PromptCommand — inject into
                            // the conversation as a user message and let the agent respond.
                            {
                                let mut resp = cmd_ctx.last_assistant_response.lock().await;
                                resp.clear();
                            }
                            let _ = input_tui_tx.send(TuiEvent::GenerationStarted).await;
                            if let Err(e) = agent.process_message(&prompt).await {
                                tracing::error!("insights agent error: {e}");
                            }
                            let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete).await;
                        }
                        SkillOutput::Text(t) | SkillOutput::Markdown(t) => {
                            let _ = input_tui_tx
                                .send(TuiEvent::TextDelta(format!("\n{t}\n")))
                                .await;
                            let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete).await;
                        }
                        SkillOutput::Error(e) => {
                            let _ = input_tui_tx
                                .send(TuiEvent::TextDelta(format!("\nError: {e}\n")))
                                .await;
                            let _ = input_tui_tx.send(TuiEvent::SlashCommandComplete).await;
                        }
                    }
                    continue;
                }

                // Not a known slash command — falls through to agent as normal input
            }

            // Clear last response buffer for /copy
            {
                let mut resp = cmd_ctx.last_assistant_response.lock().await;
                resp.clear();
            }
            // CRIT-06: Fire UserPromptSubmit hook before processing
            agent
                .fire_hook(
                    archon_core::hooks::HookType::UserPromptSubmit,
                    serde_json::json!({
                        "hook_event": "UserPromptSubmit",
                        "prompt_length": input.len(),
                    }),
                )
                .await;
            // Signal the TUI that generation is starting BEFORE the agent runs.
            // This is the canonical place is_generating gets set to true.
            let _ = input_tui_tx.send(TuiEvent::GenerationStarted).await;
            if let Err(e) = agent.process_message(&input).await {
                tracing::error!("agent loop error: {e}");
            }
            // Persist messages to session store for /resume
            let messages = &agent.conversation_state().messages;
            for (idx, msg) in messages.iter().enumerate() {
                if let Ok(json_str) = serde_json::to_string(msg)
                    && let Err(e) = session_store_for_input.save_message(
                        &session_id_for_input,
                        idx as u64,
                        &json_str,
                    )
                {
                    tracing::warn!("save_message failed at idx {idx}: {e}");
                }
            }
        }

        // CRIT-06: Fire Stop hook when the input channel closes (session ending)
        agent
            .fire_hook(
                archon_core::hooks::HookType::Stop,
                serde_json::json!({
                    "hook_event": "Stop",
                    "reason": "session_end",
                }),
            )
            .await;
    });

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
                            let _ = tui_tx.send(TuiEvent::BtwResponse(response)).await;
                        }
                        Err(e) => {
                            let _ = tui_tx
                                .send(TuiEvent::BtwResponse(format!("Error: {e}")))
                                .await;
                        }
                    }
                });
            }
        });
    }

    // Apply vim mode from config before blocking on TUI
    if config.tui.vim_mode {
        let _ = tui_event_tx.send(TuiEvent::SetVimMode(true)).await;
    }

    // Run the TUI (blocks until user quits)
    run_tui(
        tui_event_rx,
        user_input_tx,
        splash_opt,
        Some(btw_tx),
        Some(perm_prompt_tx),
    )
    .await?;

    // ── Phase 2: Graceful MCP shutdown ──────────────────────────
    mcp_manager.shutdown_all().await;
    tracing::info!("MCP servers shut down");

    Ok(())
}

// ---------------------------------------------------------------------------
// CLI-220: Tool filtering helper
// ---------------------------------------------------------------------------

/// Build the active LLM provider from the `[llm]` config section.
///
/// Matches on `llm_cfg.provider` to construct the appropriate provider.
/// Falls back to Anthropic when the selected provider is missing required
/// credentials or is unrecognised.
fn build_llm_provider(llm_cfg: &LlmConfig, api_client: AnthropicClient) -> Arc<dyn LlmProvider> {
    use archon_llm::providers::{
        AnthropicProvider, BedrockProvider, LocalProvider, OpenAiProvider, VertexProvider,
    };

    match llm_cfg.provider.as_str() {
        "openai" => {
            let key = llm_cfg.openai.api_key.clone().unwrap_or_default();
            let resolved = OpenAiProvider::resolve_api_key(&key);
            if resolved.is_empty() {
                tracing::warn!("OpenAI selected but no API key found; falling back to Anthropic");
                return Arc::new(AnthropicProvider::new(api_client));
            }
            Arc::new(OpenAiProvider::new(
                key,
                llm_cfg.openai.base_url.clone(),
                llm_cfg.openai.model.clone(),
            ))
        }
        "bedrock" => {
            if llm_cfg.bedrock.region.is_empty() || llm_cfg.bedrock.model_id.is_empty() {
                tracing::warn!(
                    "Bedrock selected but region/model_id missing; falling back to Anthropic"
                );
                return Arc::new(AnthropicProvider::new(api_client));
            }
            Arc::new(BedrockProvider::new(
                llm_cfg.bedrock.region.clone(),
                llm_cfg.bedrock.model_id.clone(),
            ))
        }
        "vertex" => {
            let project_id = llm_cfg.vertex.project_id.as_deref().unwrap_or("");
            if project_id.is_empty() {
                tracing::warn!("Vertex selected but project_id missing; falling back to Anthropic");
                return Arc::new(AnthropicProvider::new(api_client));
            }
            let publisher = if llm_cfg.vertex.model.contains("claude") {
                "anthropic"
            } else {
                "google"
            };
            Arc::new(VertexProvider::new(
                project_id.to_string(),
                llm_cfg.vertex.region.clone(),
                llm_cfg.vertex.model.clone(),
                publisher.to_string(),
                llm_cfg.vertex.credentials_file.clone(),
            ))
        }
        "local" => Arc::new(LocalProvider::new(
            llm_cfg.local.base_url.clone(),
            llm_cfg.local.model.clone(),
            llm_cfg.local.timeout_secs,
            llm_cfg.local.pull_if_missing,
        )),
        _ => {
            // Default / "anthropic" / unknown → Anthropic
            if llm_cfg.provider != "anthropic" {
                tracing::warn!(
                    "Unknown LLM provider '{}'; falling back to Anthropic",
                    llm_cfg.provider
                );
            }
            Arc::new(AnthropicProvider::new(api_client))
        }
    }
}

/// Apply `--tools` (whitelist) and `--disallowed-tools` (blacklist) from
/// resolved CLI flags to the tool registry.
fn apply_tool_filters(
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

// ---------------------------------------------------------------------------
// Slash command shared state
// ---------------------------------------------------------------------------

/// Groups all shared state needed by slash command handlers so we do not need
/// a dozen individual function parameters.
struct SlashCommandContext {
    fast_mode_shared: Arc<AtomicBool>,
    effort_level_shared: Arc<tokio::sync::Mutex<EffortLevel>>,
    model_override_shared: Arc<tokio::sync::Mutex<String>>,
    default_model: String,
    show_thinking: Arc<AtomicBool>,
    session_stats: Arc<tokio::sync::Mutex<SessionStats>>,
    permission_mode: Arc<tokio::sync::Mutex<String>>,
    session_id: String,
    cost_config: archon_core::config::CostConfig,
    memory: Arc<dyn MemoryTrait>,
    mcp_manager: McpServerManager,
    working_dir: PathBuf,
    /// Additional working directories added via `/add-dir`.
    extra_dirs: Arc<tokio::sync::Mutex<Vec<PathBuf>>>,
    auth_label: String,
    config_path: PathBuf,
    env_vars: ArchonEnvVars,
    config_sources: archon_core::config_source::ConfigSourceMap,
    skill_registry: Arc<archon_core::skills::SkillRegistry>,
    last_assistant_response: Arc<tokio::sync::Mutex<String>>,
    /// Pre-computed character count of all system prompt blocks (for /context).
    system_prompt_chars: usize,
    /// Pre-computed character count of all tool definition JSON (for /context).
    tool_defs_chars: usize,
    /// Whether `--allow-dangerously-skip-permissions` was passed (unlocks bypassPermissions mode).
    allow_bypass_permissions: bool,
    /// Shared denial log for `/denials` display.
    denial_log: Arc<tokio::sync::Mutex<archon_permissions::denial_log::DenialLog>>,
}

/// Handle slash commands. Returns `true` if the command was recognized and handled.
async fn handle_slash_command(
    input: &str,
    fast_mode: &mut FastModeState,
    effort_state: &mut EffortState,
    tui_tx: &tokio::sync::mpsc::Sender<TuiEvent>,
    ctx: &mut SlashCommandContext,
) -> bool {
    match input {
        "/fast" => {
            let new_state = fast_mode.toggle();
            ctx.fast_mode_shared.store(new_state, Ordering::Relaxed);
            let msg = if new_state {
                "Fast mode ENABLED. Responses will be faster but lower quality."
            } else {
                "Fast mode DISABLED. Back to normal quality."
            };
            let _ = tui_tx.send(TuiEvent::TextDelta(format!("\n{msg}\n"))).await;
            true
        }
        // /compact and /clear are handled inline in the input processor (need agent access)
        "/compact" | "/clear" => true,
        s if s == "/export" || s.starts_with("/export ") => true,
        "/thinking on" | "/thinking" => {
            ctx.show_thinking.store(true, Ordering::Relaxed);
            let _ = tui_tx.send(TuiEvent::ThinkingToggle(true)).await;
            let _ = tui_tx
                .send(TuiEvent::TextDelta("\nThinking display enabled.\n".into()))
                .await;
            true
        }
        "/thinking off" => {
            ctx.show_thinking.store(false, Ordering::Relaxed);
            let _ = tui_tx.send(TuiEvent::ThinkingToggle(false)).await;
            let _ = tui_tx
                .send(TuiEvent::TextDelta("\nThinking display disabled.\n".into()))
                .await;
            true
        }
        // ── /effort ────────────────────────────────────────────
        s if s.starts_with("/effort") => {
            let level_str = s.strip_prefix("/effort").unwrap_or("").trim();
            if level_str.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!(
                        "\nCurrent effort level: {}\nUsage: /effort <high|medium|low>\n",
                        effort_state.level()
                    )))
                    .await;
            } else {
                match archon_tools::validation::validate_effort_level(level_str) {
                    Ok(validated) => {
                        // Safe: validated is always one of "high", "medium", "low"
                        let level = effort::parse_level(&validated)
                            .expect("validated effort level must parse");
                        effort_state.set_level(level);
                        *ctx.effort_level_shared.lock().await = level;
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta(format!(
                                "\nEffort level set to {level}.\n"
                            )))
                            .await;
                    }
                    Err(msg) => {
                        let _ = tui_tx.send(TuiEvent::Error(msg)).await;
                    }
                }
            }
            true
        }
        // ── /model ─────────────────────────────────────────────
        s if s.starts_with("/model") => {
            let model_str = s.strip_prefix("/model").unwrap_or("").trim();
            if model_str.is_empty() {
                let current = {
                    let ov = ctx.model_override_shared.lock().await;
                    if ov.is_empty() {
                        ctx.default_model.clone()
                    } else {
                        ov.clone()
                    }
                };
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!(
                        "\nCurrent model: {current}\nUsage: /model <name>\nShortcuts: opus, sonnet, haiku\n"
                    )))
                    .await;
            } else {
                match archon_tools::validation::validate_model_name(model_str) {
                    Ok(resolved) => {
                        *ctx.model_override_shared.lock().await = resolved.clone();
                        let _ = tui_tx.send(TuiEvent::ModelChanged(resolved.clone())).await;
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta(format!(
                                "\nModel switched to {resolved}.\n"
                            )))
                            .await;
                    }
                    Err(msg) => {
                        let _ = tui_tx.send(TuiEvent::Error(msg)).await;
                    }
                }
            }
            true
        }
        // ── /copy ───────────────────────────────────────────────
        "/copy" => {
            // Find the last assistant message content
            let last_response = ctx.last_assistant_response.lock().await;
            if last_response.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(
                        "\nNo assistant response to copy.\n".into(),
                    ))
                    .await;
            } else {
                // Detect clipboard tool by trying each directly
                let tool = if std::process::Command::new("which")
                    .arg("xclip")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                {
                    "xclip"
                } else if std::process::Command::new("which")
                    .arg("clip.exe")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                {
                    "clip.exe"
                } else if std::process::Command::new("which")
                    .arg("pbcopy")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                {
                    "pbcopy"
                } else {
                    "none"
                };

                let copied = match tool {
                    "xclip" => {
                        let mut child = std::process::Command::new("xclip")
                            .arg("-selection")
                            .arg("clipboard")
                            .stdin(std::process::Stdio::piped())
                            .spawn();
                        if let Ok(ref mut c) = child {
                            use std::io::Write;
                            if let Some(ref mut stdin) = c.stdin {
                                let _ = stdin.write_all(last_response.as_bytes());
                            }
                            let _ = c.wait();
                            true
                        } else {
                            false
                        }
                    }
                    "clip.exe" => {
                        let mut child = std::process::Command::new("clip.exe")
                            .stdin(std::process::Stdio::piped())
                            .spawn();
                        if let Ok(ref mut c) = child {
                            use std::io::Write;
                            if let Some(ref mut stdin) = c.stdin {
                                let _ = stdin.write_all(last_response.as_bytes());
                            }
                            let _ = c.wait();
                            true
                        } else {
                            false
                        }
                    }
                    "pbcopy" => {
                        let mut child = std::process::Command::new("pbcopy")
                            .stdin(std::process::Stdio::piped())
                            .spawn();
                        if let Ok(ref mut c) = child {
                            use std::io::Write;
                            if let Some(ref mut stdin) = c.stdin {
                                let _ = stdin.write_all(last_response.as_bytes());
                            }
                            let _ = c.wait();
                            true
                        } else {
                            false
                        }
                    }
                    _ => false,
                };

                if copied {
                    let chars = last_response.len();
                    let _ = tui_tx
                        .send(TuiEvent::TextDelta(format!(
                            "\nCopied {chars} characters to clipboard.\n"
                        )))
                        .await;
                } else {
                    let _ = tui_tx.send(TuiEvent::Error(
                        "No clipboard tool found. Install xclip (Linux), or use clip.exe (WSL) / pbcopy (macOS).".into()
                    )).await;
                }
            }
            true
        }
        // ── /context ────────────────────────────────────────────
        "/context" => {
            let stats = ctx.session_stats.lock().await;
            let input_k = stats.input_tokens as f64 / 1000.0;
            let output_k = stats.output_tokens as f64 / 1000.0;

            // Estimate token counts from character sizes (~4 chars per token)
            let sys_prompt_tokens = ctx.system_prompt_chars as f64 / 4.0;
            let tool_def_tokens = ctx.tool_defs_chars as f64 / 4.0;

            // Conversation tokens: input tokens minus the fixed overhead
            // (system prompt + tools are sent every turn, so the last
            // input_tokens from the API already includes them).
            let fixed_overhead = sys_prompt_tokens + tool_def_tokens;
            let conversation_tokens = if stats.input_tokens > 0 {
                (stats.input_tokens as f64).max(fixed_overhead) - fixed_overhead
            } else {
                0.0
            };

            // Total estimated context = fixed overhead + conversation
            let total_context = fixed_overhead + conversation_tokens;

            let context_limit = 200_000.0_f64;
            let pct = (total_context / context_limit * 100.0).min(100.0);
            let bar_width = 40usize;
            let filled = (pct / 100.0 * bar_width as f64) as usize;
            let bar: String = format!(
                "[{}{}] {pct:.1}%",
                "#".repeat(filled),
                "-".repeat(bar_width.saturating_sub(filled))
            );

            // Format a token count nicely (e.g. 3.2k or 312)
            let fmt_tok = |t: f64| -> String {
                if t >= 1000.0 {
                    format!("{:.1}k", t / 1000.0)
                } else {
                    format!("{:.0}", t)
                }
            };

            let msg = format!(
                "\nContext window usage:\n\
                 {bar}\n\
                 \n\
                 System prompt:    ~{sys} tokens\n\
                 Tool definitions: ~{tools} tokens\n\
                 Conversation:     ~{conv} tokens\n\
                 Total context:    ~{total} / {limit}k tokens\n\
                 \n\
                 API usage this session:\n\
                 Input:  {input_k:.1}k tokens\n\
                 Output: {output_k:.1}k tokens\n\
                 Turns:  {turns}\n",
                sys = fmt_tok(sys_prompt_tokens),
                tools = fmt_tok(tool_def_tokens),
                conv = fmt_tok(conversation_tokens),
                total = fmt_tok(total_context),
                limit = context_limit as u64 / 1000,
                turns = stats.turn_count,
            );
            let _ = tui_tx.send(TuiEvent::TextDelta(msg)).await;
            true
        }
        // ── /status ────────────────────────────────────────────
        "/status" => {
            let stats = ctx.session_stats.lock().await;
            let current_model = {
                let ov = ctx.model_override_shared.lock().await;
                if ov.is_empty() {
                    ctx.default_model.clone()
                } else {
                    ov.clone()
                }
            };
            let perm_mode = ctx.permission_mode.lock().await;
            let fast = ctx.fast_mode_shared.load(Ordering::Relaxed);
            let effort = ctx.effort_level_shared.lock().await;
            let thinking_visible = ctx.show_thinking.load(Ordering::Relaxed);
            let thinking_str = if thinking_visible {
                "visible"
            } else {
                "hidden"
            };
            let in_k = stats.input_tokens as f64 / 1000.0;
            let out_k = stats.output_tokens as f64 / 1000.0;
            let msg = format!(
                "\n\
                 Model: {current_model}\n\
                 Mode: {perm_mode} (permissions)\n\
                 Fast mode: {fast_label}\n\
                 Effort: {effort}\n\
                 Thinking: {thinking_str}\n\
                 Session: {sid}\n\
                 Tokens: {in_k:.1}k in / {out_k:.1}k out\n\
                 Turns: {turns}\n",
                fast_label = if fast { "on" } else { "off" },
                effort = *effort,
                sid = &ctx.session_id[..8.min(ctx.session_id.len())],
                turns = stats.turn_count,
            );
            let _ = tui_tx.send(TuiEvent::TextDelta(msg)).await;
            true
        }
        // ── /cost ──────────────────────────────────────────────
        "/cost" => {
            let stats = ctx.session_stats.lock().await;
            let input_cost = stats.input_tokens as f64 * 3.0 / 1_000_000.0;
            let output_cost = stats.output_tokens as f64 * 15.0 / 1_000_000.0;
            let total = input_cost + output_cost;
            let warn = ctx.cost_config.warn_threshold;
            let hard = ctx.cost_config.hard_limit;
            let hard_label = if hard <= 0.0 {
                "$0.00 (disabled)".to_string()
            } else {
                format!("${hard:.2}")
            };
            let cache_line = stats.cache_stats.format_for_cost();
            let msg = format!(
                "\n\
                 Session cost: ${total:.2}\n\
                 Input tokens: {input_tok} (${input_cost:.2})\n\
                 Output tokens: {output_tok} (${output_cost:.2})\n\
                 {cache_line}\n\
                 Warn threshold: ${warn:.2}\n\
                 Hard limit: {hard_label}\n",
                input_tok = stats.input_tokens,
                output_tok = stats.output_tokens,
            );
            let _ = tui_tx.send(TuiEvent::TextDelta(msg)).await;
            true
        }
        // ── /permissions ───────────────────────────────────────
        s if s.starts_with("/permissions") => {
            let arg = s.strip_prefix("/permissions").unwrap_or("").trim();
            if arg.is_empty() {
                let mode = ctx.permission_mode.lock().await;
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!(
                        "\nCurrent permission mode: {mode}\n\
                         Usage: /permissions <mode>\n\
                         Modes: default, acceptEdits, plan, auto, dontAsk, bypassPermissions\n\
                         Legacy aliases: ask -> default, yolo -> bypassPermissions\n"
                    )))
                    .await;
            } else {
                match archon_tools::validation::validate_permission_mode(arg) {
                    Ok(resolved)
                        if resolved == "bypassPermissions" && !ctx.allow_bypass_permissions =>
                    {
                        let _ = tui_tx
                            .send(TuiEvent::Error(
                                "bypassPermissions requires --allow-dangerously-skip-permissions flag".into(),
                            ))
                            .await;
                    }
                    Ok(resolved) => {
                        *ctx.permission_mode.lock().await = resolved.clone();
                        let _ = tui_tx
                            .send(TuiEvent::PermissionModeChanged(resolved.clone()))
                            .await;
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta(format!(
                                "\nPermission mode set to {resolved}.\n"
                            )))
                            .await;
                    }
                    Err(msg) => {
                        let _ = tui_tx.send(TuiEvent::Error(msg)).await;
                    }
                }
            }
            true
        }
        // ── /config [key] [value] ──────────────────────────────
        s if s == "/config" || s.starts_with("/config ") => {
            handle_config_command(s, tui_tx, ctx).await;
            true
        }
        // ── /memory [subcommand] ───────────────────────────────
        s if s == "/memory" || s.starts_with("/memory ") => {
            handle_memory_command(s, tui_tx, &ctx.memory).await;
            true
        }
        // ── /doctor ────────────────────────────────────────────
        "/doctor" => {
            handle_doctor_command(tui_tx, ctx).await;
            true
        }
        // ── /bug ───────────────────────────────────────────────
        "/bug" => {
            let _ = tui_tx
                .send(TuiEvent::TextDelta(
                    "\nReport bugs at https://github.com/anthropics/archon/issues\n".into(),
                ))
                .await;
            true
        }
        // ── /diff ──────────────────────────────────────────────
        "/diff" => {
            handle_diff_command(tui_tx, &ctx.working_dir).await;
            true
        }
        // ── /denials ──────────────────────────────────────────
        "/denials" => {
            let log = ctx.denial_log.lock().await;
            let text = log.format_display(20);
            let _ = tui_tx
                .send(TuiEvent::TextDelta(format!("\n{text}\n")))
                .await;
            true
        }
        // ── /login ─────────────────────────────────────────────
        "/login" => {
            let cred_path = dirs::home_dir()
                .unwrap_or_default()
                .join(".claude")
                .join(".credentials.json");
            let mut msg = String::from("\nAuthentication status:\n");
            msg.push_str(&format!("  Method: {}\n", ctx.auth_label));
            if cred_path.exists() {
                msg.push_str(&format!("  Credentials: {}\n", cred_path.display()));
                msg.push_str("  Status: authenticated\n\n");
                msg.push_str("  To re-authenticate, run in another terminal:\n");
                msg.push_str("    archon login\n");
            } else {
                msg.push_str("  Credentials: not found\n");
                msg.push_str("  Status: using API key or not authenticated\n\n");
                msg.push_str("  To authenticate with OAuth:\n");
                msg.push_str("    1. Exit this session (Ctrl+D)\n");
                msg.push_str("    2. Run: archon login\n");
                msg.push_str("    3. Follow the browser flow\n");
                msg.push_str("    4. Restart archon\n");
            }
            let _ = tui_tx.send(TuiEvent::TextDelta(msg)).await;
            true
        }
        // ── /vim ───────────────────────────────────────────────
        "/vim" => {
            let _ = tui_tx.send(TuiEvent::VimToggle).await;
            let _ = tui_tx
                .send(TuiEvent::TextDelta(
                    "\nVim mode toggled. To persist: set vim_mode = true under [tui] in config.toml\n".into(),
                ))
                .await;
            true
        }
        // ── /usage ────────────────────────────────────────────
        "/usage" => {
            // Same as /cost but with more detail — redirect
            let stats = ctx.session_stats.lock().await;
            let input_cost = stats.input_tokens as f64 * 3.0 / 1_000_000.0;
            let output_cost = stats.output_tokens as f64 * 15.0 / 1_000_000.0;
            let total = input_cost + output_cost;
            let cache_line = stats.cache_stats.format_for_cost();
            let msg = format!(
                "\nUsage summary:\n\
                 Turns:         {turns}\n\
                 Input tokens:  {inp} (${input_cost:.4})\n\
                 Output tokens: {out} (${output_cost:.4})\n\
                 {cache_line}\n\
                 Total cost:    ${total:.4}\n",
                turns = stats.turn_count,
                inp = stats.input_tokens,
                out = stats.output_tokens,
            );
            let _ = tui_tx.send(TuiEvent::TextDelta(msg)).await;
            true
        }
        // ── /tasks ────────────────────────────────────────────
        "/tasks" => {
            let tasks = archon_tools::task_manager::TASK_MANAGER.list_tasks();
            if tasks.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta("\nNo background tasks.\n".into()))
                    .await;
            } else {
                let mut out = format!("\n{} background tasks:\n", tasks.len());
                for t in &tasks {
                    out.push_str(&format!("  {} [{}] {}\n", &t.id, t.status, t.description));
                }
                let _ = tui_tx.send(TuiEvent::TextDelta(out)).await;
            }
            true
        }
        // ── /release-notes ────────────────────────────────────
        "/release-notes" => {
            let _ = tui_tx
                .send(TuiEvent::TextDelta(
                    "\nArchon CLI v0.1.0 (Phase 3)\n\n\
                 - 33 tasks implemented across 7 batches\n\
                 - TUI with markdown rendering, syntax highlighting, vim mode\n\
                 - MCP stdio + HTTP transports with lifecycle management\n\
                 - Memory graph with HNSW vector search\n\
                 - 46 slash commands, hook system, config hot-reload\n\
                 - Background sessions, task tools, worktree support\n\
                 - Permission model with 6 modes\n\
                 - Print mode (-p) for scripting\n\
                 - /btw side questions with parallel API calls\n\n\
                 Full changelog: https://github.com/archon-cli/archon/releases\n"
                        .into(),
                ))
                .await;
            true
        }
        // ── /reload ───────────────────────────────────────────
        "/reload" => {
            // Force config reload from disk
            match archon_core::config_watcher::force_reload(
                std::slice::from_ref(&ctx.config_path),
                &archon_core::config::ArchonConfig::default(),
            ) {
                Ok((_new_cfg, changed)) => {
                    if changed.is_empty() {
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta(
                                "\nConfig reloaded. No changes detected.\n".into(),
                            ))
                            .await;
                    } else {
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta(format!(
                                "\nConfig reloaded. Changed: {}\n",
                                changed.join(", ")
                            )))
                            .await;
                    }
                }
                Err(e) => {
                    let _ = tui_tx
                        .send(TuiEvent::Error(format!("Config reload failed: {e}")))
                        .await;
                }
            }
            true
        }
        // ── /logout ───────────────────────────────────────────
        "/logout" => {
            // Clear OAuth credentials file
            let cred_path = dirs::home_dir()
                .unwrap_or_default()
                .join(".claude")
                .join(".credentials.json");
            if cred_path.exists() {
                match std::fs::remove_file(&cred_path) {
                    Ok(()) => {
                        let _ = tui_tx.send(TuiEvent::TextDelta(
                            "\nLogged out. Credentials cleared.\nRestart and run /login to re-authenticate.\n".into()
                        )).await;
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("Failed to clear credentials: {e}")))
                            .await;
                    }
                }
            } else {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(
                        "\nNo stored credentials found. Using API key auth.\n".into(),
                    ))
                    .await;
            }
            true
        }
        // ── /help ──────────────────────────────────────────────
        s if s == "/help" || s.starts_with("/help ") => {
            let arg = s.strip_prefix("/help").unwrap_or("").trim();
            if arg.is_empty() {
                let mut help_text = "\n\
                    Core commands:\n\
                    /model <name>        - Switch model (opus, sonnet, haiku, or full name)\n\
                    /fast                - Toggle fast mode\n\
                    /effort <level>      - Set effort (high, medium, low)\n\
                    /thinking on|off     - Show/hide thinking output\n\
                    /compact             - Trigger context compaction\n\
                    /clear               - Clear conversation history\n\
                    /status              - Show current session info\n\
                    /cost                - Show session cost breakdown\n\
                    /permissions [mode]  - Show/set permission mode (6 modes + aliases)\n\
                    /config [key] [val]  - List, get, or set runtime config values\n\
                    /memory [subcmd]     - List, search, or clear memories\n\
                    /doctor              - Run diagnostics on all subsystems\n\
                    /export              - Export conversation as JSON\n\
                    /diff                - Show git diff --stat for the working directory\n\
                    /help                - Show this help\n\
                    /help <command>      - Show detailed help for a command\n\n\
                    Extended commands:\n"
                    .to_string();
                let skill_help = ctx.skill_registry.format_help();
                help_text.push_str(&skill_help);
                let _ = tui_tx.send(TuiEvent::TextDelta(help_text)).await;
            } else {
                // Strip leading '/' from the argument if present
                let name = arg.strip_prefix('/').unwrap_or(arg);
                if let Some(detail) = ctx.skill_registry.format_skill_help(name) {
                    let _ = tui_tx
                        .send(TuiEvent::TextDelta(format!("\n{detail}\n")))
                        .await;
                } else {
                    let _ = tui_tx
                        .send(TuiEvent::Error(format!("Unknown command: /{name}")))
                        .await;
                }
            }
            true
        }
        // ── /rename ─────────────────────────────────────────────
        s if s.starts_with("/rename") => {
            let name_arg = s.strip_prefix("/rename").unwrap_or("").trim();
            if name_arg.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::Error("Usage: /rename <name>".into()))
                    .await;
            } else {
                let db_path = archon_session::storage::default_db_path();
                match archon_session::storage::SessionStore::open(&db_path) {
                    Ok(store) => {
                        match archon_session::naming::set_session_name(
                            &store,
                            &ctx.session_id,
                            name_arg,
                        ) {
                            Ok(()) => {
                                let _ = tui_tx
                                    .send(TuiEvent::SessionRenamed(name_arg.to_string()))
                                    .await;
                                let _ = tui_tx
                                    .send(TuiEvent::TextDelta(format!(
                                        "\nSession renamed to: {name_arg}\n"
                                    )))
                                    .await;
                            }
                            Err(e) => {
                                let _ = tui_tx
                                    .send(TuiEvent::Error(format!("Rename failed: {e}")))
                                    .await;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("Session store error: {e}")))
                            .await;
                    }
                }
            }
            true
        }
        // ── /resume ─────────────────────────────────────────────
        s if s.starts_with("/resume") => {
            let arg = s.strip_prefix("/resume").unwrap_or("").trim();
            let db_path = archon_session::storage::default_db_path();
            match archon_session::storage::SessionStore::open(&db_path) {
                Ok(store) => {
                    if arg.is_empty() {
                        // Show interactive session picker
                        let query = archon_session::search::SessionSearchQuery::default();
                        match archon_session::search::search_sessions(&store, &query) {
                            Ok(results) => {
                                if results.is_empty() {
                                    let _ = tui_tx
                                        .send(TuiEvent::TextDelta(
                                            "\nNo previous sessions found.\n".into(),
                                        ))
                                        .await;
                                } else {
                                    let entries: Vec<archon_tui::app::SessionPickerEntry> = results
                                        .iter()
                                        .map(|m| archon_tui::app::SessionPickerEntry {
                                            id: m.id.clone(),
                                            name: m.name.clone().unwrap_or_default(),
                                            turns: m.message_count / 2,
                                            cost: m.total_cost,
                                            last_active: m.last_active.chars().take(10).collect(),
                                        })
                                        .collect();
                                    let _ = tui_tx.send(TuiEvent::ShowSessionPicker(entries)).await;
                                }
                            }
                            Err(e) => {
                                let _ = tui_tx
                                    .send(TuiEvent::Error(format!("Search failed: {e}")))
                                    .await;
                            }
                        }
                    } else {
                        // Try to resolve by name or ID prefix
                        match archon_session::naming::resolve_by_name(&store, arg) {
                            Ok(Some(meta)) => {
                                let _ = tui_tx
                                    .send(TuiEvent::TextDelta(format!(
                                        "\nSession found: {}\nRestart with: archon --resume {}\n",
                                        meta.id, meta.id
                                    )))
                                    .await;
                            }
                            Ok(None) => {
                                let _ = tui_tx
                                    .send(TuiEvent::TextDelta(format!(
                                        "\nNo session matching '{arg}'. Use /sessions to list.\n"
                                    )))
                                    .await;
                            }
                            Err(e) => {
                                let _ = tui_tx
                                    .send(TuiEvent::Error(format!("Lookup failed: {e}")))
                                    .await;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tui_tx
                        .send(TuiEvent::Error(format!("Session store error: {e}")))
                        .await;
                }
            }
            true
        }
        // ── /mcp (MCP server manager overlay) ─────────────────
        "/mcp" => {
            let info = ctx.mcp_manager.get_server_info().await;
            let mut entries: Vec<archon_tui::app::McpServerEntry> = Vec::new();
            for (name, state, disabled) in info {
                let state_str = if disabled {
                    "disabled"
                } else {
                    match state {
                        archon_mcp::types::ServerState::Ready => "ready",
                        archon_mcp::types::ServerState::Starting
                        | archon_mcp::types::ServerState::Restarting => "starting",
                        archon_mcp::types::ServerState::Crashed => "crashed",
                        archon_mcp::types::ServerState::Stopped => "stopped",
                    }
                };
                let tools = if state_str == "ready" {
                    ctx.mcp_manager.list_tools_for(&name).await
                } else {
                    Vec::new()
                };
                entries.push(archon_tui::app::McpServerEntry {
                    name: name.clone(),
                    state: state_str.to_string(),
                    tool_count: tools.len(),
                    disabled,
                    tools,
                });
            }
            let _ = tui_tx.send(TuiEvent::ShowMcpManager(entries)).await;
            true
        }
        // ── /fork (branch conversation) ────────────────────────
        s if s == "/fork" || s.starts_with("/fork ") => {
            let name_arg = s.strip_prefix("/fork").unwrap_or("").trim();
            let db_path = archon_session::storage::default_db_path();
            match archon_session::storage::SessionStore::open(&db_path) {
                Ok(store) => {
                    let fork_name = if name_arg.is_empty() {
                        None
                    } else {
                        Some(name_arg)
                    };
                    match archon_session::fork::fork_session(&store, &ctx.session_id, fork_name) {
                        Ok(new_id) => {
                            let _ = tui_tx.send(TuiEvent::TextDelta(
                                format!("\nConversation forked as: {new_id}\nResume with: archon --resume {new_id}\nOriginal session: {}\n", ctx.session_id)
                            )).await;
                        }
                        Err(e) => {
                            let _ = tui_tx
                                .send(TuiEvent::Error(format!("Fork failed: {e}")))
                                .await;
                        }
                    }
                }
                Err(e) => {
                    let _ = tui_tx
                        .send(TuiEvent::Error(format!("Session store error: {e}")))
                        .await;
                }
            }
            true
        }
        // ── /checkpoint list | /checkpoint restore <file> ──────
        s if s == "/checkpoint" || s.starts_with("/checkpoint ") => {
            let arg = s.strip_prefix("/checkpoint").unwrap_or("").trim();
            let ckpt_path = dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("archon")
                .join("checkpoints.db");
            if arg == "list" || arg.is_empty() {
                match archon_session::checkpoint::CheckpointStore::open(&ckpt_path) {
                    Ok(store) => match store.list_modified(&ctx.session_id) {
                        Ok(snapshots) if snapshots.is_empty() => {
                            let _ = tui_tx
                                .send(TuiEvent::TextDelta(
                                    "\nNo checkpoints for this session.\n".into(),
                                ))
                                .await;
                        }
                        Ok(snapshots) => {
                            let mut out = String::from("\nCheckpoints:\n");
                            for s in &snapshots {
                                out.push_str(&format!(
                                    "  turn {} | {} | {} | {}\n",
                                    s.turn_number, s.tool_name, s.file_path, s.timestamp
                                ));
                            }
                            let _ = tui_tx.send(TuiEvent::TextDelta(out)).await;
                        }
                        Err(e) => {
                            let _ = tui_tx
                                .send(TuiEvent::Error(format!("Checkpoint list error: {e}")))
                                .await;
                        }
                    },
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("Checkpoint store error: {e}")))
                            .await;
                    }
                }
            } else if let Some(file_path) = arg.strip_prefix("restore").map(|s| s.trim()) {
                if file_path.is_empty() {
                    let _ = tui_tx
                        .send(TuiEvent::Error(
                            "Usage: /checkpoint restore <file_path>".into(),
                        ))
                        .await;
                } else {
                    match archon_session::checkpoint::CheckpointStore::open(&ckpt_path) {
                        Ok(store) => match store.restore(&ctx.session_id, file_path) {
                            Ok(()) => {
                                let _ = tui_tx
                                    .send(TuiEvent::TextDelta(format!("\nRestored: {file_path}\n")))
                                    .await;
                            }
                            Err(e) => {
                                let _ = tui_tx
                                    .send(TuiEvent::Error(format!("Restore failed: {e}")))
                                    .await;
                            }
                        },
                        Err(e) => {
                            let _ = tui_tx
                                .send(TuiEvent::Error(format!("Checkpoint store error: {e}")))
                                .await;
                        }
                    }
                }
            } else {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(
                        "\nUsage: /checkpoint list | /checkpoint restore <file_path>\n".into(),
                    ))
                    .await;
            }
            true
        }
        // ── /add-dir ───────────────────────────────────────────
        s if s.starts_with("/add-dir") => {
            let path_arg = s.strip_prefix("/add-dir").unwrap_or("").trim();
            if path_arg.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::Error("Usage: /add-dir <path>".into()))
                    .await;
            } else {
                let path = std::path::PathBuf::from(path_arg);
                if path.is_dir() {
                    // Add to the shared extra directories list (visible to agent tool context)
                    ctx.extra_dirs.lock().await.push(path.clone());
                    let _ = tui_tx.send(TuiEvent::TextDelta(
                        format!("\nAdded '{}' to working directories for this session.\nFiles in this directory are now accessible.\n", path.display())
                    )).await;
                    tracing::info!(dir = %path.display(), "added working directory via /add-dir");
                } else {
                    let _ = tui_tx
                        .send(TuiEvent::Error(format!("Directory not found: {path_arg}")))
                        .await;
                }
            }
            true
        }
        // ── /color ─────────────────────────────────────────────
        s if s.starts_with("/color") => {
            let color_arg = s.strip_prefix("/color").unwrap_or("").trim();
            if color_arg.is_empty() {
                let _ = tui_tx.send(TuiEvent::TextDelta(
                    "\nAvailable accent colors: red, green, yellow, blue, magenta, cyan, white, default\n\
                     Usage: /color <name>\n".into()
                )).await;
            } else if let Some(color) = archon_tui::theme::parse_color(color_arg) {
                let _ = tui_tx.send(TuiEvent::SetAccentColor(color)).await;
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!(
                        "\nAccent color set to '{color_arg}'.\n"
                    )))
                    .await;
            } else {
                let _ = tui_tx.send(TuiEvent::Error(
                    format!("Unknown color '{color_arg}'. Available: red, green, yellow, blue, magenta, cyan, white, default")
                )).await;
            }
            true
        }
        // ── /theme ─────────────────────────────────────────────
        s if s.starts_with("/theme") => {
            let theme_arg = s.strip_prefix("/theme").unwrap_or("").trim();
            if theme_arg.is_empty() {
                let names = archon_tui::theme::available_themes().join(", ");
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!(
                        "\nAvailable themes: {names}\nUsage: /theme <name>\n"
                    )))
                    .await;
            } else if archon_tui::theme::theme_by_name(theme_arg).is_some() {
                let _ = tui_tx.send(TuiEvent::SetTheme(theme_arg.to_string())).await;
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!(
                        "\nTheme set to '{theme_arg}'.\n"
                    )))
                    .await;
            } else {
                let names = archon_tui::theme::available_themes().join(", ");
                let _ = tui_tx
                    .send(TuiEvent::Error(format!(
                        "Unknown theme '{theme_arg}'. Available: {names}"
                    )))
                    .await;
            }
            true
        }
        // ── /recall ────────────────────────────────────────────
        s if s.starts_with("/recall") => {
            let query = s.strip_prefix("/recall").unwrap_or("").trim();
            if query.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::Error(
                        "Usage: /recall <query> — search memories by keyword".into(),
                    ))
                    .await;
            } else {
                // Search the memory graph
                let results = ctx.memory.recall_memories(query, 10);
                match results {
                    Ok(memories) => {
                        if memories.is_empty() {
                            let _ = tui_tx
                                .send(TuiEvent::TextDelta(format!(
                                    "\nNo memories found for '{query}'.\n"
                                )))
                                .await;
                        } else {
                            let mut out =
                                format!("\n{} memories matching '{query}':\n\n", memories.len());
                            for m in &memories {
                                let title = if m.title.is_empty() {
                                    "(untitled)"
                                } else {
                                    &m.title
                                };
                                let snippet: String = m.content.chars().take(100).collect();
                                let id_short = &m.id[..8.min(m.id.len())];
                                out.push_str(&format!(
                                    "  [{id_short}] {title}\n    {snippet}...\n\n"
                                ));
                            }
                            let _ = tui_tx.send(TuiEvent::TextDelta(out)).await;
                        }
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("Memory search failed: {e}")))
                            .await;
                    }
                }
            }
            true
        }
        // ── /rules — list, edit, remove behavioral rules (CRIT-14 ITEM 4) ──
        s if s == "/rules" || s.starts_with("/rules ") => {
            let args_str = s.strip_prefix("/rules").unwrap_or("").trim();
            let engine = RulesEngine::new(ctx.memory.as_ref());
            if args_str.is_empty() || args_str == "list" {
                match engine.get_rules_sorted() {
                    Ok(rules) if rules.is_empty() => {
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta("\nNo behavioral rules.\n".into()))
                            .await;
                    }
                    Ok(rules) => {
                        let mut out = format!("\n{} behavioral rules:\n\n", rules.len());
                        for r in &rules {
                            let id_short = &r.id[..8.min(r.id.len())];
                            out.push_str(&format!(
                                "  [{id_short}] (score: {:.1}) {}\n",
                                r.score, r.text
                            ));
                        }
                        let _ = tui_tx.send(TuiEvent::TextDelta(out)).await;
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("rules list failed: {e}")))
                            .await;
                    }
                }
            } else if let Some(rest) = args_str.strip_prefix("edit ") {
                // /rules edit <id> <new text>
                let parts: Vec<&str> = rest.splitn(2, ' ').collect();
                if parts.len() < 2 {
                    let _ = tui_tx
                        .send(TuiEvent::Error("Usage: /rules edit <id> <new text>".into()))
                        .await;
                } else {
                    let id_prefix = parts[0];
                    let new_text = parts[1];
                    // Resolve full ID from prefix
                    match engine.get_rules_sorted() {
                        Ok(rules) => {
                            if let Some(rule) = rules.iter().find(|r| r.id.starts_with(id_prefix)) {
                                match engine.update_rule(&rule.id, new_text) {
                                    Ok(()) => {
                                        let _ = tui_tx
                                            .send(TuiEvent::TextDelta(format!(
                                                "\nRule updated: {new_text}\n"
                                            )))
                                            .await;
                                    }
                                    Err(e) => {
                                        let _ = tui_tx
                                            .send(TuiEvent::Error(format!(
                                                "update_rule failed: {e}"
                                            )))
                                            .await;
                                    }
                                }
                            } else {
                                let _ = tui_tx
                                    .send(TuiEvent::Error(format!(
                                        "No rule matching ID prefix '{id_prefix}'"
                                    )))
                                    .await;
                            }
                        }
                        Err(e) => {
                            let _ = tui_tx
                                .send(TuiEvent::Error(format!("rules lookup failed: {e}")))
                                .await;
                        }
                    }
                }
            } else if let Some(id_prefix) = args_str.strip_prefix("remove ") {
                let id_prefix = id_prefix.trim();
                match engine.get_rules_sorted() {
                    Ok(rules) => {
                        if let Some(rule) = rules.iter().find(|r| r.id.starts_with(id_prefix)) {
                            match engine.remove_rule(&rule.id) {
                                Ok(()) => {
                                    let _ = tui_tx
                                        .send(TuiEvent::TextDelta(format!(
                                            "\nRule removed: {}\n",
                                            rule.text
                                        )))
                                        .await;
                                }
                                Err(e) => {
                                    let _ = tui_tx
                                        .send(TuiEvent::Error(format!("remove_rule failed: {e}")))
                                        .await;
                                }
                            }
                        } else {
                            let _ = tui_tx
                                .send(TuiEvent::Error(format!(
                                    "No rule matching ID prefix '{id_prefix}'"
                                )))
                                .await;
                        }
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("rules lookup failed: {e}")))
                            .await;
                    }
                }
            } else {
                let _ = tui_tx
                    .send(TuiEvent::Error(
                        "Usage: /rules [list | edit <id> <text> | remove <id>]".into(),
                    ))
                    .await;
            }
            true
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// /config handler
// ---------------------------------------------------------------------------

async fn handle_config_command(
    input: &str,
    tui_tx: &tokio::sync::mpsc::Sender<TuiEvent>,
    ctx: &SlashCommandContext,
) {
    let args: Vec<&str> = input
        .strip_prefix("/config")
        .unwrap_or_default()
        .trim()
        .splitn(2, ' ')
        .collect();
    let key = args.first().map(|s| s.trim()).unwrap_or("");
    let value = args.get(1).map(|s| s.trim()).unwrap_or("");

    if key == "sources" {
        let output = archon_core::config_source::format_sources(&ctx.config_sources);
        if output.is_empty() {
            let _ = tui_tx
                .send(TuiEvent::TextDelta("\nNo config sources tracked.\n".into()))
                .await;
        } else {
            let _ = tui_tx
                .send(TuiEvent::TextDelta(format!("\nConfig sources:\n{output}")))
                .await;
        }
        return;
    }

    if key.is_empty() {
        // List all config keys with current values
        let keys = archon_tools::config_tool::all_keys();
        let mut lines = String::from("\nRuntime configuration:\n");
        for k in &keys {
            let val = archon_tools::config_tool::get_config_value(k)
                .unwrap_or_else(|| "(unknown)".into());
            lines.push_str(&format!("  {k} = {val}\n"));
        }
        let _ = tui_tx.send(TuiEvent::TextDelta(lines)).await;
    } else if value.is_empty() {
        // Get a single key
        match archon_tools::config_tool::get_config_value(key) {
            Some(val) => {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!("\n{key} = {val}\n")))
                    .await;
            }
            None => {
                let _ = tui_tx
                    .send(TuiEvent::Error(format!("Unknown config key: {key}")))
                    .await;
            }
        }
    } else {
        // Set key=value via the ConfigTool
        use archon_tools::tool::{AgentMode, ToolContext};
        let tool = archon_tools::config_tool::ConfigTool;
        let ctx = ToolContext {
            working_dir: std::env::current_dir().unwrap_or_default(),
            session_id: String::new(),
            mode: AgentMode::Normal,
            extra_dirs: Vec::new(),
        };
        let result = archon_tools::tool::Tool::execute(
            &tool,
            serde_json::json!({"action": "set", "key": key, "value": value}),
            &ctx,
        )
        .await;
        if result.is_error {
            let _ = tui_tx.send(TuiEvent::Error(result.content)).await;
        } else {
            let _ = tui_tx
                .send(TuiEvent::TextDelta(format!("\n{}\n", result.content)))
                .await;
        }
    }
}

// ---------------------------------------------------------------------------
// /memory handler
// ---------------------------------------------------------------------------

async fn handle_memory_command(
    input: &str,
    tui_tx: &tokio::sync::mpsc::Sender<TuiEvent>,
    memory: &Arc<dyn MemoryTrait>,
) {
    let rest = input.strip_prefix("/memory").unwrap_or("").trim();
    let (subcmd, arg) = match rest.split_once(' ') {
        Some((s, a)) => (s.trim(), a.trim()),
        None => (rest, ""),
    };

    match subcmd {
        "" | "list" => match memory.list_recent(10) {
            Ok(memories) if memories.is_empty() => {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta("\nNo memories stored.\n".into()))
                    .await;
            }
            Ok(memories) => {
                let mut out = format!("\nRecent memories ({}):\n", memories.len());
                for m in &memories {
                    let short_id = &m.id[..8.min(m.id.len())];
                    let date = m.created_at.format("%Y-%m-%d %H:%M");
                    out.push_str(&format!(
                        "  [{short_id}] {title} ({mtype}, {date})\n",
                        title = m.title,
                        mtype = m.memory_type,
                    ));
                }
                let _ = tui_tx.send(TuiEvent::TextDelta(out)).await;
            }
            Err(e) => {
                let _ = tui_tx
                    .send(TuiEvent::Error(format!("Memory graph error: {e}")))
                    .await;
            }
        },
        "search" => {
            if arg.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::Error("Usage: /memory search <query>".into()))
                    .await;
                return;
            }
            match memory.recall_memories(arg, 10) {
                Ok(results) if results.is_empty() => {
                    let _ = tui_tx
                        .send(TuiEvent::TextDelta(format!(
                            "\nNo memories matching \"{arg}\".\n"
                        )))
                        .await;
                }
                Ok(results) => {
                    let mut out = format!("\nMemories matching \"{arg}\" ({}):\n", results.len());
                    for m in &results {
                        let short_id = &m.id[..8.min(m.id.len())];
                        out.push_str(&format!(
                            "  [{short_id}] {title} -- {snippet}\n",
                            title = m.title,
                            snippet = truncate_str(&m.content, 80),
                        ));
                    }
                    let _ = tui_tx.send(TuiEvent::TextDelta(out)).await;
                }
                Err(e) => {
                    let _ = tui_tx
                        .send(TuiEvent::Error(format!("Memory search error: {e}")))
                        .await;
                }
            }
        }
        "clear" => match memory.clear_all() {
            Ok(n) => {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!(
                        "\nCleared {n} memories from the graph.\n"
                    )))
                    .await;
            }
            Err(e) => {
                let _ = tui_tx
                    .send(TuiEvent::Error(format!("Failed to clear memories: {e}")))
                    .await;
            }
        },
        other => {
            let _ = tui_tx
                .send(TuiEvent::Error(format!(
                    "Unknown memory subcommand: {other}. Use list, search, or clear."
                )))
                .await;
        }
    }
}

/// Truncate a string to at most `max` bytes, appending "..." if truncated.
/// Safe for multi-byte UTF-8: always splits on a char boundary.
fn truncate_str(s: &str, max: usize) -> String {
    let trimmed = s.replace('\n', " ");
    if trimmed.len() <= max {
        trimmed
    } else {
        let mut end = max.saturating_sub(3);
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &trimmed[..end])
    }
}

// ---------------------------------------------------------------------------
// /doctor handler
// ---------------------------------------------------------------------------

async fn handle_doctor_command(
    tui_tx: &tokio::sync::mpsc::Sender<TuiEvent>,
    ctx: &SlashCommandContext,
) {
    let mut out = String::from("\nArchon diagnostics:\n");

    // Auth
    out.push_str(&format!("  Auth: authenticated ({})\n", ctx.auth_label));

    // MCP servers
    let states = ctx.mcp_manager.get_server_states().await;
    if states.is_empty() {
        out.push_str("  MCP servers: none configured\n");
    } else {
        out.push_str(&format!("  MCP servers: {} configured\n", states.len()));
        for (name, state) in &states {
            out.push_str(&format!("    {name}: {state}\n"));
        }
    }

    // Memory graph
    match ctx.memory.memory_count() {
        Ok(count) => out.push_str(&format!("  Memory graph: open ({count} memories)\n")),
        Err(e) => out.push_str(&format!("  Memory graph: error ({e})\n")),
    }

    // Config
    let config_valid = ctx.config_path.exists();
    out.push_str(&format!(
        "  Config: {} ({})\n",
        ctx.config_path.display(),
        if config_valid { "valid" } else { "not found" },
    ));

    // Checkpoint store
    let ckpt_path = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("archon")
        .join("checkpoints.db");
    let ckpt_status = if ckpt_path.exists() { "open" } else { "closed" };
    out.push_str(&format!("  Checkpoint store: {ckpt_status}\n"));

    // Model
    let current_model = {
        let ov = ctx.model_override_shared.lock().await;
        if ov.is_empty() {
            ctx.default_model.clone()
        } else {
            ov.clone()
        }
    };
    out.push_str(&format!("  Model: {current_model}\n"));

    // Environment variables
    out.push_str(&env_vars::format_doctor_env_vars(&ctx.env_vars));

    let _ = tui_tx.send(TuiEvent::TextDelta(out)).await;
}

// ---------------------------------------------------------------------------
// /diff handler
// ---------------------------------------------------------------------------

async fn handle_diff_command(tui_tx: &tokio::sync::mpsc::Sender<TuiEvent>, working_dir: &PathBuf) {
    let result = tokio::process::Command::new("git")
        .arg("diff")
        .arg("--stat")
        .current_dir(working_dir)
        .output()
        .await;

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !output.status.success() {
                if stderr.contains("not a git repository") {
                    let _ = tui_tx
                        .send(TuiEvent::TextDelta("\nNot in a git repository.\n".into()))
                        .await;
                } else {
                    let _ = tui_tx
                        .send(TuiEvent::Error(format!("git diff failed: {stderr}")))
                        .await;
                }
                return;
            }
            if stdout.is_empty() {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta("\nNo uncommitted changes.\n".into()))
                    .await;
            } else {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!("\n{stdout}")))
                    .await;
            }
        }
        Err(e) => {
            let _ = tui_tx
                .send(TuiEvent::Error(format!("Failed to run git: {e}")))
                .await;
        }
    }
}

/// Fetch account UUID from Anthropic OAuth profile endpoint.
async fn fetch_account_uuid(auth: &archon_llm::auth::AuthProvider) -> String {
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

#[cfg(test)]
mod wire_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strip_cache_control_noop_when_enabled() {
        let mut blocks = vec![
            json!({"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}}),
            json!({"type": "text", "text": "b"}),
        ];
        strip_cache_control_if_disabled(&mut blocks, true);
        assert!(blocks[0].get("cache_control").is_some());
        assert!(blocks[1].get("cache_control").is_none());
    }

    #[test]
    fn strip_cache_control_removes_key_when_disabled() {
        let mut blocks = vec![
            json!({"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}}),
            json!({"type": "text", "text": "b", "cache_control": {"type": "ephemeral", "scope": "org"}}),
            json!({"type": "text", "text": "c"}),
        ];
        strip_cache_control_if_disabled(&mut blocks, false);
        assert!(blocks[0].get("cache_control").is_none());
        assert!(blocks[1].get("cache_control").is_none());
        assert!(blocks[2].get("cache_control").is_none());
        // Text content preserved
        assert_eq!(blocks[0].get("text").unwrap(), "a");
        assert_eq!(blocks[1].get("text").unwrap(), "b");
        assert_eq!(blocks[2].get("text").unwrap(), "c");
    }
}
