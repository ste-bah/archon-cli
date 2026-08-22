// Setup and initialization functions extracted from main.rs.
//
// TASK #228 (post-cargo-fix): several helpers below are not currently
// invoked from main.rs but are kept as ready-to-use utilities for the
// in-progress runtime/launcher refactor. File-level allow(dead_code)
// silences the noise without flagging each individually.
#![allow(dead_code)]

use std::path::PathBuf;

use anyhow::Result;

use archon_core::config::default_config_path;
use archon_core::config_layers::ConfigLayer;
use archon_core::env_vars::{self, ArchonEnvVars};
use archon_core::logging::{init_logging, resolve_log_dir};

use crate::cli_args::Cli;

/// Parse `--setting-sources` names into [`ConfigLayer`] variants, warning on
/// unrecognised values.
pub fn parse_layer_filter(sources: &[String]) -> Vec<ConfigLayer> {
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
pub fn strip_cache_control_if_disabled(
    blocks: &mut [serde_json::Value],
    prompt_cache_enabled: bool,
) {
    if prompt_cache_enabled {
        return;
    }
    for block in blocks.iter_mut() {
        if let Some(obj) = block.as_object_mut() {
            obj.remove("cache_control");
        }
    }
}

/// Initialize logging system and return the log directory.
/// The log guard is stored internally and will be dropped when the function returns,
/// but that's acceptable since the logging system is already initialized.
///
/// `ARCHON_DEBUG_LOG_DIR` wins over `ARCHON_LOG_DIR`, being the more deliberate
/// of the two. It was parsed into `ArchonEnvVars::debug_log_dir` and printed by
/// the env dump, but nothing anywhere read it — so setting it did nothing at
/// all, silently, while appearing in the documented list of knobs.
pub fn setup_logging(session_id: &str, log_level: &str) -> Result<PathBuf> {
    let log_dir = resolve_log_dir();

    init_logging(session_id, log_level, &log_dir)
        .map_err(|e| anyhow::anyhow!("logging init failed: {e}"))?;

    Ok(log_dir)
}

/// Resolve CLI flags and apply them to config (model override, log level, etc.).
/// Returns the resolved flags for later use.
pub fn resolve_cli_flags(
    cli: &Cli,
    config: &mut archon_core::config::ArchonConfig,
) -> archon_core::cli_flags::ResolvedFlags {
    use archon_core::cli_flags::resolve_flags;

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

    resolved_flags
}

// A second `setup_voice_pipeline` lived here, byte-for-byte the same wiring as
// the one in `command/tui_helpers.rs` except for the `yield_now`, and nothing
// called it: `main.rs` calls the other one. Two copies of a subsystem's
// startup path is one copy that can be fixed while the running one stays
// broken, which is exactly what had happened — both built a mock audio source
// (#192).

/// Load environment variables and warn about unrecognized ARCHON_* vars.
pub fn load_env_vars() -> ArchonEnvVars {
    let env_vars = env_vars::load_env_vars();
    let all_env: std::collections::HashMap<String, String> = std::env::vars().collect();
    let unrecognized = env_vars::warn_unrecognized_archon_vars(
        &all_env,
        crate::command::trading_data::trading_data_env::TRADING_ENV_VARS,
    );
    for var_name in &unrecognized {
        eprintln!("warning: unrecognized environment variable: {var_name}");
    }
    env_vars
}

/// Load and merge config from file, CLI settings, and environment overrides.
pub fn load_config(
    env_vars: &ArchonEnvVars,
    cli: &Cli,
) -> (archon_core::config::ArchonConfig, std::path::PathBuf) {
    let config_path = env_vars
        .config_dir
        .as_ref()
        .map(|d| d.join("config.toml"))
        .unwrap_or_else(default_config_path);

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
    env_vars::apply_env_overrides(&mut config, env_vars);

    // #178: the cost estimator is reachable from the TUI event loop, the status
    // line and two slash commands, none of which carry configuration. This is
    // the one point where the merged config exists and every one of them is
    // downstream, so `[context.model_pricing]` is installed here.
    archon_core::cost::install_pricing_overrides(config.context.model_pricing.clone());

    (config, config_path)
}

/// Log startup information about memory and prompt cache settings.
pub fn log_startup_info(config: &archon_core::config::ArchonConfig, session_id: &str) {
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

/// Generate a new session ID.
pub fn generate_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
