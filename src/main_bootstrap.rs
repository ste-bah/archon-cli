use anyhow::Result;
use archon_core::cli_flags::{ResolvedFlags, resolve_flags};
use archon_core::config::default_config_path;
use archon_core::config_layers::ConfigLayer;
use archon_core::env_vars::{self, ArchonEnvVars};
use archon_core::logging::{LogGuard, default_log_dir, init_logging, rotate_logs};

use crate::cli_args::Cli;

pub(crate) struct MainBootstrap {
    pub(crate) env_vars: ArchonEnvVars,
    pub(crate) config: archon_core::config::ArchonConfig,
    pub(crate) resolved_flags: ResolvedFlags,
    pub(crate) working_dir_for_config: std::path::PathBuf,
    pub(crate) session_id: String,
    _log_guard: LogGuard,
}

pub(crate) fn bootstrap(cli: &Cli) -> Result<MainBootstrap> {
    apply_remote_url_env(cli);
    let env_vars = env_vars::load_env_vars();
    warn_unrecognized_archon_vars();
    let working_dir_for_config = std::env::current_dir().unwrap_or_default();
    let mut config = load_config(cli, &env_vars, &working_dir_for_config);
    env_vars::apply_env_overrides(&mut config, &env_vars);
    let resolved_flags = resolve_flags(&cli.to_flag_input()).unwrap_or_else(|error| {
        eprintln!("error: {error}");
        std::process::exit(1);
    });
    apply_cli_logging_and_model_overrides(&mut config, &resolved_flags);
    let session_id = uuid::Uuid::new_v4().to_string();
    let log_guard = init_session_logging(&session_id, &config);
    log_startup_state(&session_id, &config);
    Ok(MainBootstrap {
        env_vars,
        config,
        resolved_flags,
        working_dir_for_config,
        session_id,
        _log_guard: log_guard,
    })
}

fn apply_remote_url_env(cli: &Cli) {
    if let Some(ref url) = cli.remote_url {
        // SAFETY: this runs during single-threaded startup before Tokio work is spawned.
        unsafe {
            std::env::set_var("ARCHON_REMOTE_URL", url);
        }
    }
}

fn warn_unrecognized_archon_vars() {
    let all_env: std::collections::HashMap<String, String> = std::env::vars().collect();
    let unrecognized = env_vars::warn_unrecognized_archon_vars(&all_env);
    for var_name in &unrecognized {
        eprintln!("warning: unrecognized environment variable: {var_name}");
    }
}

fn load_config(
    cli: &Cli,
    env_vars: &ArchonEnvVars,
    working_dir: &std::path::Path,
) -> archon_core::config::ArchonConfig {
    let config_path = env_vars
        .config_dir
        .as_ref()
        .map(|dir| dir.join("config.toml"))
        .unwrap_or_else(default_config_path);
    let layer_filter: Option<Vec<ConfigLayer>> = cli
        .setting_sources
        .as_ref()
        .map(|sources| crate::setup::parse_layer_filter(sources));
    archon_core::config_layers::load_layered_config(
        Some(&config_path),
        working_dir,
        cli.settings.as_deref(),
        layer_filter.as_deref(),
    )
    .unwrap_or_else(|error| {
        eprintln!("warning: failed to load config, using defaults: {error}");
        archon_core::config::ArchonConfig::default()
    })
}

fn apply_cli_logging_and_model_overrides(
    config: &mut archon_core::config::ArchonConfig,
    resolved_flags: &ResolvedFlags,
) {
    if let Some(ref model) = resolved_flags.model {
        config.api.default_model = model.clone();
    }
    if resolved_flags.verbose {
        config.logging.level = "trace".to_string();
    }
    if let Some(ref filter) = resolved_flags.debug {
        config.logging.level = match filter {
            Some(categories) => format!(
                "warn,{}",
                categories
                    .split(',')
                    .map(|category| format!("{category}=debug"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            None => "debug".to_string(),
        };
    }
    if let Ok(log_level) = std::env::var("ARCHON_LOG")
        && !log_level.trim().is_empty()
    {
        config.logging.level = log_level.trim().to_string();
    }
}

fn init_session_logging(session_id: &str, config: &archon_core::config::ArchonConfig) -> LogGuard {
    let log_dir = std::env::var_os("ARCHON_LOG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_log_dir);
    let log_guard =
        init_logging(session_id, &config.logging.level, &log_dir).unwrap_or_else(|error| {
            eprintln!("fatal: logging init failed: {error}");
            std::process::exit(1);
        });
    if let Err(error) = rotate_logs(&log_dir, config.logging.max_files) {
        tracing::warn!("failed to rotate logs: {error}");
    }
    tracing::debug!(
        "logging: max_files={}, max_file_size_mb={}",
        config.logging.max_files,
        config.logging.max_file_size_mb,
    );
    log_guard
}

fn log_startup_state(session_id: &str, config: &archon_core::config::ArchonConfig) {
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
}
