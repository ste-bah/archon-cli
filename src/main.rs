mod agent_handle;
pub(crate) mod session;
pub(crate) mod setup;
mod slash_context;
pub(crate) mod cli_args;
mod command;
mod runtime;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use clap::Parser;

use archon_consciousness::assembler::{AssemblyInput, BudgetConfig, SystemPromptAssembler};
use archon_consciousness::defaults::load_configured_defaults;
use archon_consciousness::rules::RulesEngine;
use archon_core::agent::{Agent, AgentConfig, AgentEvent, SessionStats, TimestampedEvent};
use archon_core::agents::AgentRegistry;
use archon_core::cli_flags::resolve_flags;
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
use archon_mcp::lifecycle::McpServerManager;
use archon_memory::{MemoryAccess, MemoryGraph, MemoryTrait};
use archon_permissions::auto::{AutoModeConfig, AutoModeEvaluator};
use archon_tui::app::TuiEvent;

use cli_args::{Cli, Commands};
use crate::runtime::llm::build_llm_provider;

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
    // TODO(TUI-330): app::TuiEvent moves to archon_tui::events::TuiEvent
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
            use crate::command::update::handle_update_command;
            return handle_update_command(check, force, &config).await;
        }
        Some(Commands::Remote { .. }) | Some(Commands::Serve { .. }) => {
            use crate::command::remote::handle_remote_command;
            return handle_remote_command(&cli, &config).await;
        }
        Some(Commands::Team { action }) => {
            use crate::command::team::handle_team_command;
            return handle_team_command(&action, &config, &env_vars).await;
        }
        Some(Commands::IdeStdio) => {
            use crate::command::ide_stdio::handle_ide_stdio_command;
            return handle_ide_stdio_command().await;
        }
        Some(Commands::Web { port, bind_address, no_open }) => {
            use crate::command::web::handle_web_command;
            return handle_web_command(port, bind_address, no_open, &config).await;
        }
        Some(Commands::Pipeline { action }) => {
            use crate::command::pipeline::handle_pipeline_command;
            return handle_pipeline_command(&action, &config, &env_vars).await;
        }
        Some(Commands::RunAgentAsync { name, input, version, detach }) => {
            use crate::command::task::handle_run_agent_async;
            return handle_run_agent_async(name, input, version, detach, &working_dir_for_config).await;
        }
        Some(Commands::TaskStatus { task_id, watch }) => {
            use crate::command::task::handle_task_status;
            return handle_task_status(&task_id, watch, &working_dir_for_config).await;
        }
        Some(Commands::TaskResult { task_id, stream }) => {
            use crate::command::task::handle_task_result;
            return handle_task_result(&task_id, stream, &working_dir_for_config).await;
        }
        Some(Commands::TaskCancel { task_id }) => {
            use crate::command::task::handle_task_cancel;
            return handle_task_cancel(&task_id, &working_dir_for_config).await;
        }
        Some(Commands::TaskList { state, agent, since }) => {
            use crate::command::task::handle_task_list;
            return handle_task_list(state, agent, since, &working_dir_for_config).await;
        }
        Some(Commands::TaskEvents { task_id, from_seq }) => {
            use crate::command::task::handle_task_events;
            return handle_task_events(&task_id, from_seq, &working_dir_for_config).await;
        }
        Some(Commands::Metrics) => {
            use crate::command::task::handle_metrics;
            return handle_metrics(&working_dir_for_config).await;
        }
        Some(Commands::AgentList { include_invalid }) => {
            use crate::command::agent::handle_agent_list;
            return handle_agent_list(include_invalid, &working_dir_for_config).await;
        }
        Some(Commands::AgentSearch {
            tags,
            capabilities,
            name_pattern,
            version,
            logic,
            include_invalid,
            registry_url,
        }) => {
            use crate::command::agent::handle_agent_search;
            return handle_agent_search(
                tags, capabilities, name_pattern, version, logic,
                include_invalid, registry_url, &working_dir_for_config,
            ).await;
        }
        Some(Commands::AgentInfo { name, version, json }) => {
            use crate::command::agent::handle_agent_info;
            return handle_agent_info(name, version, json, &working_dir_for_config).await;
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

        // Load user styles from ~/.archon/output-styles/
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

    let (agent_event_tx, agent_event_rx) = tokio::sync::mpsc::unbounded_channel::<TimestampedEvent>();
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

// Delegation stub - implementation moved to session module

pub(crate) async fn run_interactive_session(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    env_vars: &ArchonEnvVars,
    resume_messages: Option<Vec<serde_json::Value>>,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    voice_event_rx: Option<tokio::sync::mpsc::Receiver<archon_tui::app::TuiEvent>>,
) -> Result<()> {
    crate::session::run_interactive_session(
        config,
        session_id,
        cli,
        env_vars,
        resume_messages,
        resolved_flags,
        voice_event_rx,
    )
    .await
}



// ---------------------------------------------------------------------------
// CLI-220: Tool filtering helper
// ---------------------------------------------------------------------------

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
    garden_config: archon_memory::garden::GardenConfig,
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
    /// Agent registry for agent management skills.
    agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
    /// TASK-AGS-622: Shared command registry (constructed once at App level).
    /// Dispatch via `Registry::get` is wired in TASK-AGS-623; this task only
    /// holds the Arc.
    #[allow(dead_code)]
    registry: std::sync::Arc<crate::command::registry::Registry>,
    /// TASK-AGS-623: Shared dispatcher (parser → registry → handler).
    /// Installed as a parallel gate at the top of `handle_slash_command`
    /// (PATH A hybrid) while the legacy inline match continues to run
    /// the actual command bodies.
    #[allow(dead_code)]
    dispatcher: std::sync::Arc<crate::command::dispatcher::Dispatcher>,
}

/// Handle slash commands. Returns `true` if the command was recognized and handled.
async fn handle_slash_command(
    input: &str,
    fast_mode: &mut FastModeState,
    effort_state: &mut EffortState,
    tui_tx: &tokio::sync::mpsc::Sender<TuiEvent>,
    ctx: &mut SlashCommandContext,
) -> bool {
    // TASK-AGS-623 dispatcher gate (PATH A hybrid).
    //
    // Every slash input now flows through exactly one `Dispatcher::dispatch`
    // call: parser → registry lookup → handler (currently no-op stubs from
    // TASK-AGS-622) or `TuiEvent::Error("Unknown command: /{name}")` on miss.
    // Recognized commands fall through to the legacy 43-arm match below,
    // which continues to perform the actual command bodies until TASK-AGS-624
    // migrates those bodies into the registry's stub `execute` methods.
    // Non-slash / empty / bare-`/` inputs short-circuit with `false` — the
    // same behaviour the TASK-AGS-621 parser gate provided.
    let mut __cmd_ctx = crate::command::registry::CommandContext {
        tui_tx: tui_tx.clone(),
    };
    let _ = ctx.dispatcher.dispatch(&mut __cmd_ctx, input);
    if !ctx.dispatcher.recognizes(input) {
        return false;
    }

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
        // ── /garden ────────────────────────────────────────────
        s if s == "/garden" || s.starts_with("/garden ") => {
            let sub = s.strip_prefix("/garden").unwrap_or("").trim();
            if sub == "stats" {
                match archon_memory::garden::format_garden_stats(ctx.memory.as_ref(), 10) {
                    Ok(stats) => {
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta(format!("\n{stats}\n")))
                            .await;
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("Garden stats failed: {e}")))
                            .await;
                    }
                }
            } else {
                match archon_memory::garden::consolidate(ctx.memory.as_ref(), &ctx.garden_config) {
                    Ok(report) => {
                        let formatted = report.format();
                        let _ = tui_tx
                            .send(TuiEvent::TextDelta(format!("\n{formatted}\n")))
                            .await;
                    }
                    Err(e) => {
                        let _ = tui_tx
                            .send(TuiEvent::Error(format!("Garden consolidation failed: {e}")))
                            .await;
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
                .join(".archon")
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
                .join(".archon")
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
            ..Default::default()
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









