mod agent_handle;
pub(crate) mod cli_args;
mod command;
mod runtime;
pub(crate) mod session;
pub(crate) mod setup;
mod slash_context;

use anyhow::Result;
use clap::Parser;

use archon_core::cli_flags::resolve_flags;
use archon_core::config::default_config_path;
use archon_core::config_layers::ConfigLayer;
use archon_core::env_vars;
use archon_core::input_format::InputFormat;
use archon_core::logging::{default_log_dir, init_logging, rotate_logs};
use archon_core::output_format::OutputFormat;
use archon_core::print_mode::PrintModeConfig;

use cli_args::{Cli, Commands};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // TASK-TUI-625-FOLLOWUP: wire --remote-url to ARCHON_REMOTE_URL so
    // the /session slash command's EnvRemoteUrlProvider can see the URL
    // supplied at startup. SAFETY: set_var is unsafe on Rust 1.77+ — this
    // call runs before any other thread is spawned, so concurrent env-var
    // access cannot occur.
    if let Some(ref url) = cli.remote_url {
        unsafe {
            std::env::set_var("ARCHON_REMOTE_URL", url);
        }
    }

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
    let layer_filter: Option<Vec<ConfigLayer>> = cli
        .setting_sources
        .as_ref()
        .map(|s| crate::setup::parse_layer_filter(s));

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
    let voice_event_rx = crate::command::tui_helpers::setup_voice_pipeline(&config).await;

    // Handle subcommands
    match cli.command {
        Some(Commands::Login) => {
            return crate::command::login::handle_login(&config).await;
        }
        Some(Commands::Plugin { action }) => {
            return crate::command::plugin::handle_plugin_command(action);
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
        Some(Commands::Web {
            port,
            bind_address,
            no_open,
        }) => {
            use crate::command::web::handle_web_command;
            return handle_web_command(port, bind_address, no_open, &config).await;
        }
        Some(Commands::Pipeline { action }) => {
            use crate::command::pipeline::handle_pipeline_command;
            return handle_pipeline_command(&action, &config, &env_vars).await;
        }
        Some(Commands::RunAgentAsync {
            name,
            input,
            version,
            detach,
        }) => {
            use crate::command::task::handle_run_agent_async;
            return handle_run_agent_async(name, input, version, detach, &working_dir_for_config)
                .await;
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
        Some(Commands::TaskList {
            state,
            agent,
            since,
        }) => {
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
                tags,
                capabilities,
                name_pattern,
                version,
                logic,
                include_invalid,
                registry_url,
                &working_dir_for_config,
            )
            .await;
        }
        Some(Commands::AgentInfo {
            name,
            version,
            json,
        }) => {
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
        return crate::command::tui_helpers::handle_list_output_styles();
    }

    // ── Theme: --list-themes (CLI-315) ───────────────────────────
    if cli.list_themes {
        return crate::command::tui_helpers::handle_list_themes(&cli, &config);
    }

    // Handle --resume with no ID: list recent sessions and exit
    if let Some(None) = &cli.resume {
        return crate::session::handle_resume_list().await;
    }

    // For --resume with ID, load the session messages to restore
    let mut resume_messages = if let Some(Some(ref id)) = cli.resume {
        Some(crate::session::load_resume_messages(id)?)
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
        return crate::command::sessions::handle_sessions(&cli);
    }

    // ── Background sessions (CLI-221) ─────────────────────────
    if cli.ps {
        return crate::command::background::handle_bg_list();
    }
    if let Some(ref id) = cli.kill_session {
        return crate::command::background::handle_bg_kill(id);
    }
    if let Some(ref id) = cli.attach {
        return crate::command::background::handle_bg_attach(id);
    }
    if let Some(ref id) = cli.logs {
        return crate::command::background::handle_bg_logs(id);
    }
    if cli.bg.is_some() {
        return crate::command::background::handle_bg_launch(&cli);
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
        let exit_code = crate::session::run_print_mode_session(
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
    crate::session::run_interactive_session(
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
        crate::setup::strip_cache_control_if_disabled(&mut blocks, true);
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
        crate::setup::strip_cache_control_if_disabled(&mut blocks, false);
        assert!(blocks[0].get("cache_control").is_none());
        assert!(blocks[1].get("cache_control").is_none());
        assert!(blocks[2].get("cache_control").is_none());
        // Text content preserved
        assert_eq!(blocks[0].get("text").unwrap(), "a");
        assert_eq!(blocks[1].get("text").unwrap(), "b");
        assert_eq!(blocks[2].get("text").unwrap(), "c");
    }
}
