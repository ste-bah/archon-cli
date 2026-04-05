//! Centralized environment variable handling for Archon CLI.
//!
//! All `ARCHON_*` and `ANTHROPIC_*` environment variables are read once at
//! startup into an [`ArchonEnvVars`] struct. No scattered `std::env::var()`
//! calls elsewhere in the codebase.
//!
//! Precedence: CLI flags > env vars > config file > hardcoded defaults.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::config::ArchonConfig;

// ---------------------------------------------------------------------------
// Known variables — single source of truth
// ---------------------------------------------------------------------------

/// All recognized environment variable names. Used for unrecognized-var
/// detection. Keep sorted alphabetically within each group.
pub const KNOWN_ARCHON_VARS: &[&str] = &[
    // Auth
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ARCHON_API_KEY",
    "ARCHON_OAUTH_TOKEN",
    // Model & behavior
    "ARCHON_MODEL",
    "ARCHON_EFFORT",
    "ARCHON_PERMISSION_MODE",
    "ARCHON_IDENTITY_MODE",
    // Feature control
    "ARCHON_SIMPLE",
    "ARCHON_DISABLE_HOOKS",
    "ARCHON_DISABLE_MEMORY",
    "ARCHON_DISABLE_PERSONALITY",
    // Debugging
    "ARCHON_DEBUG",
    "ARCHON_DEBUG_LOG_DIR",
    "ARCHON_VERBOSE",
    "ARCHON_LOG",
    // Paths
    "ARCHON_CONFIG_DIR",
    "ARCHON_DATA_DIR",
    // Telemetry (recognized but no-op)
    "ARCHON_DISABLE_TELEMETRY",
];

// ---------------------------------------------------------------------------
// Parsed env var struct
// ---------------------------------------------------------------------------

/// All environment-variable-derived configuration, parsed once at startup.
#[derive(Clone)]
pub struct ArchonEnvVars {
    // Auth
    pub anthropic_api_key: Option<String>,
    pub archon_api_key: Option<String>,
    pub archon_oauth_token: Option<String>,

    // Model & behavior
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
    pub identity_mode: Option<String>,

    // Feature control
    /// Bare mode (--bare). Consumed by CLI-220 (CLI flags expansion).
    pub simple: bool,
    /// Skip all hooks. Consumed by CLI-224 (hook system).
    pub disable_hooks: bool,
    pub disable_memory: bool,
    /// Skip personality injection. Consumed by CLI-220 (CLI flags expansion).
    pub disable_personality: bool,

    // Debugging
    pub debug: bool,
    /// Override log directory. Consumed by CLI-220 (CLI flags expansion).
    pub debug_log_dir: Option<String>,
    pub verbose: bool,

    // Paths
    pub config_dir: Option<PathBuf>,
    /// Override data directory. Consumed by CLI-220 (CLI flags expansion).
    pub data_dir: Option<PathBuf>,

    // Telemetry (no-op — telemetry is always off)
    pub disable_telemetry: bool,
}

impl std::fmt::Debug for ArchonEnvVars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchonEnvVars")
            .field(
                "anthropic_api_key",
                &self.anthropic_api_key.as_deref().map(mask_secret),
            )
            .field(
                "archon_api_key",
                &self.archon_api_key.as_deref().map(mask_secret),
            )
            .field(
                "archon_oauth_token",
                &self.archon_oauth_token.as_deref().map(mask_secret),
            )
            .field("model", &self.model)
            .field("effort", &self.effort)
            .field("permission_mode", &self.permission_mode)
            .field("identity_mode", &self.identity_mode)
            .field("simple", &self.simple)
            .field("disable_hooks", &self.disable_hooks)
            .field("disable_memory", &self.disable_memory)
            .field("disable_personality", &self.disable_personality)
            .field("debug", &self.debug)
            .field("debug_log_dir", &self.debug_log_dir)
            .field("verbose", &self.verbose)
            .field("config_dir", &self.config_dir)
            .field("data_dir", &self.data_dir)
            .field("disable_telemetry", &self.disable_telemetry)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Parse a boolean from an optional env var value.
///
/// Truthy: `1`, `true`, `yes` (case-insensitive).
/// Falsy: `0`, `false`, `no`, empty string, `None`, anything else.
pub fn parse_bool_env(value: Option<String>) -> bool {
    match value {
        None => false,
        Some(v) => matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"),
    }
}

/// Read an optional string var, treating empty/whitespace-only as `None`.
fn read_optional_string(env: &HashMap<String, String>, key: &str) -> Option<String> {
    env.get(key)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Read an optional path var, treating empty/whitespace-only as `None`.
fn read_optional_path(env: &HashMap<String, String>, key: &str) -> Option<PathBuf> {
    read_optional_string(env, key).map(PathBuf::from)
}

/// Read a boolean var from the map.
fn read_bool(env: &HashMap<String, String>, key: &str) -> bool {
    parse_bool_env(env.get(key).cloned())
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Load all environment variables from the process environment.
///
/// This is the production entry point. Call once at startup.
pub fn load_env_vars() -> ArchonEnvVars {
    let env: HashMap<String, String> = std::env::vars().collect();
    load_env_vars_from(&env)
}

/// Load environment variables from an explicit map.
///
/// Testable variant — no process-global side effects.
pub fn load_env_vars_from(env: &HashMap<String, String>) -> ArchonEnvVars {
    ArchonEnvVars {
        // Auth
        anthropic_api_key: read_optional_string(env, "ANTHROPIC_API_KEY"),
        archon_api_key: read_optional_string(env, "ARCHON_API_KEY"),
        archon_oauth_token: read_optional_string(env, "ARCHON_OAUTH_TOKEN"),

        // Model & behavior
        model: read_optional_string(env, "ARCHON_MODEL"),
        effort: read_optional_string(env, "ARCHON_EFFORT"),
        permission_mode: read_optional_string(env, "ARCHON_PERMISSION_MODE"),
        identity_mode: read_optional_string(env, "ARCHON_IDENTITY_MODE"),

        // Feature control
        simple: read_bool(env, "ARCHON_SIMPLE"),
        disable_hooks: read_bool(env, "ARCHON_DISABLE_HOOKS"),
        disable_memory: read_bool(env, "ARCHON_DISABLE_MEMORY"),
        disable_personality: read_bool(env, "ARCHON_DISABLE_PERSONALITY"),

        // Debugging
        debug: read_bool(env, "ARCHON_DEBUG"),
        debug_log_dir: read_optional_string(env, "ARCHON_DEBUG_LOG_DIR"),
        verbose: read_bool(env, "ARCHON_VERBOSE"),

        // Paths
        config_dir: read_optional_path(env, "ARCHON_CONFIG_DIR"),
        data_dir: read_optional_path(env, "ARCHON_DATA_DIR"),

        // Telemetry
        disable_telemetry: read_bool(env, "ARCHON_DISABLE_TELEMETRY"),
    }
}

// ---------------------------------------------------------------------------
// Apply overrides to config
// ---------------------------------------------------------------------------

/// Apply environment variable overrides to the config struct.
///
/// Only touches fields that have a corresponding env var set.
/// Does NOT apply auth vars — those are handled by `resolve_auth_from_env`.
pub fn apply_env_overrides(config: &mut ArchonConfig, vars: &ArchonEnvVars) {
    // Model & behavior
    if let Some(ref model) = vars.model {
        config.api.default_model = model.clone();
    }
    if let Some(ref effort) = vars.effort {
        config.api.default_effort = effort.clone();
    }
    if let Some(ref mode) = vars.permission_mode {
        config.permissions.mode = mode.clone();
    }
    if let Some(ref mode) = vars.identity_mode {
        config.identity.mode = mode.clone();
    }

    // Feature control
    if vars.disable_memory {
        config.memory.enabled = false;
    }

    // Debugging — verbose (trace) takes precedence over debug
    if vars.verbose {
        config.logging.level = "trace".into();
    } else if vars.debug {
        config.logging.level = "debug".into();
    }
}

// ---------------------------------------------------------------------------
// Secret masking
// ---------------------------------------------------------------------------

/// Mask a secret value for display (e.g. `/doctor` output).
///
/// Keys with 8+ characters: show first 4 + `...` + last 4.
/// Shorter keys: show `****`.
pub fn mask_secret(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() < 8 {
        return "****".to_string();
    }
    let prefix: String = chars[..4].iter().collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{prefix}...{suffix}")
}

// ---------------------------------------------------------------------------
// Unrecognized variable detection
// ---------------------------------------------------------------------------

/// Find `ARCHON_*` or `ANTHROPIC_*` env vars that are not in `KNOWN_ARCHON_VARS`.
///
/// Returns the list of unrecognized variable names. Caller should log at
/// debug level.
pub fn warn_unrecognized_archon_vars(env: &HashMap<String, String>) -> Vec<String> {
    env.keys()
        .filter(|k| k.starts_with("ARCHON_") || k.starts_with("ANTHROPIC_"))
        .filter(|k| !KNOWN_ARCHON_VARS.contains(&k.as_str()))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// /doctor output
// ---------------------------------------------------------------------------

/// Format the environment variables section for `/doctor` output.
///
/// Secrets are masked. Unset vars shown as `(not set)`. The telemetry var
/// gets a special note.
pub fn format_doctor_env_vars(vars: &ArchonEnvVars) -> String {
    let mut out = String::from("  Environment variables:\n");

    // Helper closures
    let show_secret = |name: &str, val: &Option<String>| -> String {
        match val {
            Some(v) => format!("    {name}: {}\n", mask_secret(v)),
            None => format!("    {name}: (not set)\n"),
        }
    };
    let show_string = |name: &str, val: &Option<String>| -> String {
        match val {
            Some(v) => format!("    {name}: {v}\n"),
            None => format!("    {name}: (not set)\n"),
        }
    };
    let show_bool = |name: &str, val: bool| -> String {
        if val {
            format!("    {name}: true\n")
        } else {
            format!("    {name}: (not set)\n")
        }
    };
    let show_path = |name: &str, val: &Option<PathBuf>| -> String {
        match val {
            Some(p) => format!("    {name}: {}\n", p.display()),
            None => format!("    {name}: (not set)\n"),
        }
    };

    // Auth (secrets)
    out.push_str(&show_secret("ANTHROPIC_API_KEY", &vars.anthropic_api_key));
    out.push_str(&show_secret("ARCHON_API_KEY", &vars.archon_api_key));
    out.push_str(&show_secret("ARCHON_OAUTH_TOKEN", &vars.archon_oauth_token));

    // Model & behavior
    out.push_str(&show_string("ARCHON_MODEL", &vars.model));
    out.push_str(&show_string("ARCHON_EFFORT", &vars.effort));
    out.push_str(&show_string(
        "ARCHON_PERMISSION_MODE",
        &vars.permission_mode,
    ));
    out.push_str(&show_string("ARCHON_IDENTITY_MODE", &vars.identity_mode));

    // Feature control
    out.push_str(&show_bool("ARCHON_SIMPLE", vars.simple));
    out.push_str(&show_bool("ARCHON_DISABLE_HOOKS", vars.disable_hooks));
    out.push_str(&show_bool("ARCHON_DISABLE_MEMORY", vars.disable_memory));
    out.push_str(&show_bool(
        "ARCHON_DISABLE_PERSONALITY",
        vars.disable_personality,
    ));

    // Debugging
    out.push_str(&show_bool("ARCHON_DEBUG", vars.debug));
    out.push_str(&show_string("ARCHON_DEBUG_LOG_DIR", &vars.debug_log_dir));
    out.push_str(&show_bool("ARCHON_VERBOSE", vars.verbose));

    // Paths
    out.push_str(&show_path("ARCHON_CONFIG_DIR", &vars.config_dir));
    out.push_str(&show_path("ARCHON_DATA_DIR", &vars.data_dir));

    // Telemetry
    if vars.disable_telemetry {
        out.push_str(
            "    ARCHON_DISABLE_TELEMETRY: set (telemetry is permanently off -- this var is a no-op)\n",
        );
    } else {
        out.push_str("    ARCHON_DISABLE_TELEMETRY: (not set)\n");
    }

    out
}
