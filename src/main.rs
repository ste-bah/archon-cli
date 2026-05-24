#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::doc_overindented_list_items)]
#![allow(clippy::empty_line_after_doc_comments)]

mod agent_handle;
pub(crate) mod cli_args;
mod command;
mod gametheory_tool_executor;
mod main_dispatch;
#[cfg(test)]
mod main_tests;
mod panic_save;
mod runtime;
pub(crate) mod session;
pub(crate) mod session_loop;
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
    let mut cli = Cli::parse();

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

    tracing::info!(
        "Archon CLI v{} started, session {session_id}",
        env!("CARGO_PKG_VERSION")
    );
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

    gametheory_tool_executor::install(config.clone(), env_vars.clone());

    // TODO(TUI-330): app::TuiEvent moves to archon_tui::events::TuiEvent
    let voice_event_rx = crate::command::tui_helpers::setup_voice_pipeline(&config).await;

    // Handle subcommands. Remote commands inspect the full CLI, so they keep
    // the command attached; every other subcommand can be moved to the
    // dispatcher while later interactive paths keep using the remaining flags.
    if matches!(
        &cli.command,
        Some(Commands::Remote { .. }) | Some(Commands::Serve { .. })
    ) {
        return crate::command::remote::handle_remote_command(&cli, &config).await;
    }
    if let Some(command) = cli.command.take() {
        return main_dispatch::handle_subcommand(
            command,
            &cli,
            &config,
            &env_vars,
            &resolved_flags,
            &working_dir_for_config,
        )
        .await;
    }

    // ── Headless mode (--headless) ───────────────────────────────
    if cli.headless {
        let headless_session_id = cli.session_id.clone().unwrap_or_else(|| session_id.clone());
        tracing::info!("headless mode: session_id={headless_session_id}");
        let exit_code = crate::session::run_headless_session(
            &config,
            &headless_session_id,
            &cli,
            &env_vars,
            &resolved_flags,
        )
        .await;
        std::process::exit(exit_code);
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
        return crate::session::handle_resume_list_with_config(&config).await;
    }

    // For --resume with ID, load the session messages to restore
    let mut resume_messages = if let Some(Some(ref id)) = cli.resume {
        Some(crate::session::load_resume_messages_with_config(
            id, &config,
        )?)
    } else {
        None
    };

    // ── --continue: resume most recent session in this directory ──
    if cli.continue_session && resume_messages.is_none() {
        let cwd = std::env::current_dir().unwrap_or_default();
        let cwd_str = cwd.to_string_lossy().to_string();
        let db_path = crate::command::store_paths::session_db_path(&config);
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
        let db_path = crate::command::store_paths::session_db_path(&config);
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
        return crate::command::sessions::handle_sessions(&cli, &config);
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
        let json_schema = resolve_json_schema(&cli).unwrap_or_else(|e| {
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
            json_schema,
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
    if !std::io::IsTerminal::is_terminal(&std::io::stdin())
        || !std::io::IsTerminal::is_terminal(&std::io::stdout())
    {
        anyhow::bail!(
            "interactive mode requires a TTY; use -p/--print, --headless, or --ide-stdio for non-interactive input"
        );
    }

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

fn resolve_json_schema(cli: &Cli) -> Result<Option<String>> {
    if let Some(schema) = &cli.json_schema {
        return Ok(Some(schema.clone()));
    }
    let Some(path) = &cli.json_schema_path else {
        return Ok(None);
    };
    let schema = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read JSON schema from {}: {e}", path.display()))?;
    Ok(Some(schema))
}
