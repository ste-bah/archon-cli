use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("config parse error: {0}")]
    ParseError(#[from] toml::de::Error),

    #[error("config validation error: {0}")]
    ValidationError(String),
}

// ---------------------------------------------------------------------------
// Top-level config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ArchonConfig {
    pub api: ApiConfig,
    pub identity: IdentityConfig,
    pub tools: ToolsConfig,
    pub permissions: PermissionsConfig,
    pub context: ContextConfig,
    pub memory: MemoryConfig,
    pub cost: CostConfig,
    pub logging: LoggingConfig,
    pub session: SessionConfig,
    pub checkpoint: CheckpointConfig,
    pub personality: archon_consciousness::personality::PersonalityProfile,
    pub consciousness: ConsciousnessConfig,
    pub tui: TuiConfig,
}

impl Default for ArchonConfig {
    fn default() -> Self {
        Self {
            api: ApiConfig::default(),
            identity: IdentityConfig::default(),
            tools: ToolsConfig::default(),
            permissions: PermissionsConfig::default(),
            context: ContextConfig::default(),
            memory: MemoryConfig::default(),
            cost: CostConfig::default(),
            logging: LoggingConfig::default(),
            session: SessionConfig::default(),
            checkpoint: CheckpointConfig::default(),
            personality: archon_consciousness::personality::PersonalityProfile::default(),
            consciousness: ConsciousnessConfig::default(),
            tui: TuiConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Section structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiConfig {
    pub default_model: String,
    pub thinking_budget: u32,
    pub default_effort: String,
    pub max_retries: u32,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            default_model: "claude-sonnet-4-6".into(),
            thinking_budget: 16384,
            default_effort: "medium".into(),
            max_retries: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IdentityConfig {
    pub mode: String,
    pub spoof_version: String,
    pub spoof_entrypoint: String,
    pub spoof_betas: Option<Vec<String>>,
    pub attestation_hook: Option<String>,
    pub anti_distillation: bool,
    pub workload: Option<String>,
    pub custom: Option<CustomIdentityConfig>,
}

impl Default for IdentityConfig {
    fn default() -> Self {
        Self {
            mode: "clean".into(),
            spoof_version: "2.1.89".into(),
            spoof_entrypoint: "cli".into(),
            spoof_betas: None,
            attestation_hook: None,
            anti_distillation: false,
            workload: None,
            custom: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomIdentityConfig {
    pub user_agent: String,
    pub x_app: String,
    pub extra_headers: Option<HashMap<String, String>>,
}

impl Default for CustomIdentityConfig {
    fn default() -> Self {
        Self {
            user_agent: "archon-cli/0.1.0".into(),
            x_app: "archon".into(),
            extra_headers: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    pub bash_timeout: u64,
    pub bash_max_output: usize,
    pub max_concurrency: u8,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            bash_timeout: 120,
            bash_max_output: 102400,
            max_concurrency: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionsConfig {
    pub mode: String,
    pub allow_paths: Vec<String>,
    pub deny_paths: Vec<String>,
    pub safe_commands: Vec<String>,
    pub risky_commands: Vec<String>,
    pub dangerous_commands: Vec<String>,
    /// Fine-grained rules: tools/patterns that are always allowed.
    pub always_allow: Vec<archon_permissions::rules::ToolRule>,
    /// Fine-grained rules: tools/patterns that are always denied.
    pub always_deny: Vec<archon_permissions::rules::ToolRule>,
    /// Fine-grained rules: tools/patterns that always require confirmation.
    pub always_ask: Vec<archon_permissions::rules::ToolRule>,
}

impl Default for PermissionsConfig {
    fn default() -> Self {
        Self {
            mode: "default".into(),
            allow_paths: Vec::new(),
            deny_paths: Vec::new(),
            safe_commands: Vec::new(),
            risky_commands: Vec::new(),
            dangerous_commands: Vec::new(),
            always_allow: Vec::new(),
            always_deny: Vec::new(),
            always_ask: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextConfig {
    pub compact_threshold: f32,
    pub max_tokens: Option<u32>,
    pub preserve_recent_turns: u32,
    /// Whether to use prompt caching (cache_control breakpoints on static blocks).
    pub prompt_cache: bool,
    /// Maximum characters for hierarchical CLAUDE.md content.
    pub claudemd_max_tokens: u32,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            compact_threshold: 0.80,
            max_tokens: None,
            preserve_recent_turns: 3,
            prompt_cache: true,
            claudemd_max_tokens: 8192,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    pub enabled: bool,
    pub db_path: Option<String>,
    /// Embedding provider: `auto`, `local`, or `openai`.
    pub embedding_provider: archon_memory::embedding::EmbeddingProviderKind,
    /// Keyword/vector blend factor for hybrid search (0.0 = pure vector, 1.0 = pure keyword).
    pub hybrid_alpha: f32,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            db_path: None,
            embedding_provider: archon_memory::embedding::EmbeddingProviderKind::Auto,
            hybrid_alpha: 0.3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CostConfig {
    pub warn_threshold: f64,
    pub hard_limit: f64,
}

impl Default for CostConfig {
    fn default() -> Self {
        Self {
            warn_threshold: 5.0,
            hard_limit: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
    pub max_files: u32,
    pub max_file_size_mb: u32,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            max_files: 50,
            max_file_size_mb: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionConfig {
    pub db_path: Option<String>,
    pub auto_resume: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            db_path: None,
            auto_resume: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CheckpointConfig {
    pub enabled: bool,
    pub max_checkpoints: u32,
    /// Whether to snapshot files on first read (default false to avoid storage bloat).
    pub snapshot_on_read: bool,
}

impl Default for CheckpointConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_checkpoints: 10,
            snapshot_on_read: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConsciousnessConfig {
    /// Whether the inner voice feature is enabled.
    pub inner_voice: bool,
    /// Energy decay rate applied each turn (multiplied by current energy).
    pub energy_decay_rate: f32,
}

impl Default for ConsciousnessConfig {
    fn default() -> Self {
        Self {
            inner_voice: true,
            energy_decay_rate: 0.98,
        }
    }
}

/// TUI-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiConfig {
    /// Enable vim-style keybindings in the input area. Default: `false`.
    pub vim_mode: bool,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self { vim_mode: false }
    }
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

/// Returns the default config file path: `~/.config/archon/config.toml`
pub fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("archon")
        .join("config.toml")
}

/// Validate an `ArchonConfig`, returning `ConfigError::ValidationError` on
/// any invalid field values.
pub fn validate(config: &ArchonConfig) -> Result<(), ConfigError> {
    // identity.mode
    match config.identity.mode.as_str() {
        "spoof" | "clean" | "custom" => {}
        other => {
            return Err(ConfigError::ValidationError(format!(
                "identity.mode must be \"spoof\", \"clean\", or \"custom\", got \"{other}\""
            )));
        }
    }

    // permissions.mode — accepts all 6 canonical modes + legacy aliases
    if config.permissions.mode.parse::<archon_permissions::mode::PermissionMode>().is_err() {
        return Err(ConfigError::ValidationError(format!(
            "permissions.mode must be a valid mode (default, acceptEdits, plan, auto, \
             dontAsk, bypassPermissions) or legacy alias (ask, yolo), got \"{}\"",
            config.permissions.mode
        )));
    }

    // tools.bash_timeout
    if config.tools.bash_timeout == 0 {
        return Err(ConfigError::ValidationError(
            "tools.bash_timeout must be > 0".into(),
        ));
    }

    // tools.max_concurrency
    if !(1..=16).contains(&config.tools.max_concurrency) {
        return Err(ConfigError::ValidationError(format!(
            "tools.max_concurrency must be 1..=16, got {}",
            config.tools.max_concurrency
        )));
    }

    // context.compact_threshold
    if !(0.0..=1.0).contains(&config.context.compact_threshold) {
        return Err(ConfigError::ValidationError(format!(
            "context.compact_threshold must be 0.0..=1.0, got {}",
            config.context.compact_threshold
        )));
    }

    // consciousness.energy_decay_rate
    if !(0.0..=1.0).contains(&config.consciousness.energy_decay_rate) {
        return Err(ConfigError::ValidationError(format!(
            "consciousness.energy_decay_rate must be 0.0..=1.0, got {}",
            config.consciousness.energy_decay_rate
        )));
    }

    // personality profile
    config
        .personality
        .validate()
        .map_err(|e| ConfigError::ValidationError(e.to_string()))?;

    Ok(())
}

/// Load configuration from the default path. If the file does not exist,
/// create the parent directory and write a default config file.
pub fn load_config() -> Result<ArchonConfig, ConfigError> {
    load_config_from(default_config_path())
}

/// Load configuration from a specific path. If the file does not exist,
/// create the parent directory, write a default config, and return defaults.
pub fn load_config_from(path: PathBuf) -> Result<ArchonConfig, ConfigError> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let defaults = ArchonConfig::default();
        let toml_str = toml::to_string_pretty(&defaults)
            .map_err(|e| ConfigError::ValidationError(format!("failed to serialize defaults: {e}")))?;
        fs::write(&path, toml_str)?;
        return Ok(defaults);
    }

    let content = fs::read_to_string(&path)?;
    let config: ArchonConfig = toml::from_str(&content)?;
    validate(&config)?;
    Ok(config)
}
