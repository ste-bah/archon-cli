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
#[derive(Default)]
pub struct ArchonConfig {
    pub api: ApiConfig,
    pub llm: LlmConfig,
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
    /// Active output style name.  Resolved at startup against the
    /// `OutputStyleRegistry`.  Unknown values fall back to `"default"` with a
    /// warning.  Can be overridden by `--output-style` CLI flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_style: Option<String>,
    /// Self-update configuration.
    pub update: crate::update::UpdateConfig,
    /// Remote agent / SSH configuration.
    pub remote: SshRemoteConfig,
    /// WebSocket server configuration.
    #[serde(default)]
    pub ws_remote: WsRemoteConfig,
    /// Multi-agent orchestration configuration.
    #[serde(default)]
    pub orchestrator: crate::orchestrator::config::OrchestratorConfig,
    /// Voice input configuration.
    #[serde(default)]
    pub voice: VoiceConfig,
    /// Web UI configuration.
    #[serde(default)]
    pub web: WebConfig,
}

// ---------------------------------------------------------------------------
// Voice config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    pub enabled: bool,
    pub device: String,
    pub vad_threshold: f32,
    pub stt_provider: String,
    pub stt_api_key: String,
    pub stt_url: String,
    pub hotkey: String,
    pub toggle_mode: bool,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            device: "default".into(),
            vad_threshold: 0.02,
            stt_provider: "openai".into(),
            stt_api_key: String::new(),
            stt_url: "https://api.openai.com".into(),
            hotkey: "ctrl+shift+v".into(),
            toggle_mode: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Web UI config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebConfig {
    /// Port to listen on.
    pub port: u16,
    /// Address to bind. `"127.0.0.1"` = localhost only (default).
    pub bind_address: String,
    /// Open default browser automatically after server starts.
    pub open_browser: bool,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            port: 8421,
            bind_address: "127.0.0.1".to_string(),
            open_browser: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Remote / SSH config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub key_file: Option<String>,
    pub agent_forwarding: bool,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 22,
            user: String::new(),
            key_file: None,
            agent_forwarding: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SshRemoteConfig {
    pub sync_mode: String,
    pub ssh: SshConfig,
}

/// WebSocket remote server configuration stored in `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WsRemoteConfig {
    /// Port the WebSocket server listens on.
    #[serde(default = "default_ws_port")]
    pub port: u16,
    /// Path to a TLS certificate file (PEM).  `None` = no TLS.
    pub tls_cert: Option<String>,
    /// Path to a TLS private key file (PEM).  Required when `tls_cert` is set.
    pub tls_key: Option<String>,
}

fn default_ws_port() -> u16 {
    8420
}

impl Default for WsRemoteConfig {
    fn default() -> Self {
        Self {
            port: 8420,
            tls_cert: None,
            tls_key: None,
        }
    }
}

impl Default for SshRemoteConfig {
    fn default() -> Self {
        Self {
            sync_mode: "manual".to_string(),
            ssh: SshConfig::default(),
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
    /// Override the Anthropic API base URL. Useful for pointing at LiteLLM,
    /// Ollama, or any other OpenAI-compatible / Anthropic-compatible proxy.
    /// Resolution priority:
    ///   1. `ANTHROPIC_BASE_URL` env var (always wins)
    ///   2. This field in config.toml
    ///   3. Hardcoded default: `https://api.anthropic.com/v1/messages`
    pub base_url: Option<String>,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            default_model: "claude-sonnet-4-6".into(),
            thinking_budget: 16384,
            default_effort: "medium".into(),
            max_retries: 3,
            base_url: None,
        }
    }
}

/// LLM provider configuration.
///
/// Controls which backend provider is active and allows provider-specific
/// settings to be set in the `[llm]` section of `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    /// The active provider name (e.g. `"anthropic"`, `"openai"`, `"bedrock"`, `"vertex"`, `"local"`).
    pub provider: String,
    /// OpenAI provider settings.
    pub openai: LlmOpenAiConfig,
    /// AWS Bedrock provider settings.
    pub bedrock: LlmBedrockConfig,
    /// Google Vertex AI provider settings.
    pub vertex: LlmVertexConfig,
    /// Local / Ollama provider settings.
    pub local: LlmLocalConfig,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".into(),
            openai: LlmOpenAiConfig::default(),
            bedrock: LlmBedrockConfig::default(),
            vertex: LlmVertexConfig::default(),
            local: LlmLocalConfig::default(),
        }
    }
}

/// OpenAI provider settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmOpenAiConfig {
    /// API key. If `None`, resolved from `OPENAI_API_KEY` env var.
    pub api_key: Option<String>,
    /// Override the OpenAI base URL (e.g. for Azure OpenAI or a proxy).
    pub base_url: Option<String>,
    /// Default model to use.
    pub model: String,
}

impl Default for LlmOpenAiConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: None,
            model: "gpt-4o".to_string(),
        }
    }
}

/// AWS Bedrock provider settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmBedrockConfig {
    /// AWS region (e.g. `"us-east-1"`).
    pub region: String,
    /// Bedrock model ID (e.g. `"anthropic.claude-sonnet-4-20250514-v1:0"`).
    pub model_id: String,
}

impl Default for LlmBedrockConfig {
    fn default() -> Self {
        Self {
            region: "us-east-1".to_string(),
            model_id: "anthropic.claude-sonnet-4-20250514-v1:0".to_string(),
        }
    }
}

/// Google Vertex AI provider settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmVertexConfig {
    /// GCP project ID. If `None`, resolved from ADC or env var.
    pub project_id: Option<String>,
    /// GCP region (e.g. `"us-central1"`).
    pub region: String,
    /// Model name (e.g. `"claude-sonnet-4-20250514@20250514"`).
    pub model: String,
    /// Path to service account credentials JSON file.
    pub credentials_file: Option<String>,
}

impl Default for LlmVertexConfig {
    fn default() -> Self {
        Self {
            project_id: None,
            region: "us-central1".to_string(),
            model: "claude-sonnet-4-20250514@20250514".to_string(),
            credentials_file: None,
        }
    }
}

/// Local / Ollama provider settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmLocalConfig {
    /// Base URL for the OpenAI-compatible local server.
    pub base_url: String,
    /// Default model name.
    pub model: String,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// Whether to pull the model if not present (Ollama-specific).
    pub pull_if_missing: bool,
}

impl Default for LlmLocalConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:11434/v1".to_string(),
            model: "llama3:8b".to_string(),
            timeout_secs: 300,
            pull_if_missing: true,
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
}

impl Default for CheckpointConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_checkpoints: 10,
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
    /// Behavioral rules to seed into the memory graph on startup.
    /// If non-empty, these replace the built-in defaults.
    /// Idempotent: rules already present are not duplicated.
    /// Maximum 50 rules. Each must be a non-empty string.
    pub initial_rules: Vec<String>,
}

impl Default for ConsciousnessConfig {
    fn default() -> Self {
        Self {
            inner_voice: true,
            energy_decay_rate: 0.98,
            initial_rules: Vec::new(),
        }
    }
}

/// TUI-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiConfig {
    /// Enable vim-style keybindings in the input area. Default: `false`.
    pub vim_mode: bool,
    /// Default verbosity mode.  `true` = verbose (show everything), `false` = brief.
    /// Can be overridden per-session via the VerbosityToggle tool or `Ctrl+V`.
    pub verbose: bool,
    /// Named color theme.  Built-ins: intj, intp, ..., dark, light, ocean, fire,
    /// forest, mono, daltonized, auto.  Unknown names fall back to `"dark"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            vim_mode: false,
            verbose: true,
            theme: None,
        }
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
    if config
        .permissions
        .mode
        .parse::<archon_permissions::mode::PermissionMode>()
        .is_err()
    {
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

    // consciousness.initial_rules
    if config.consciousness.initial_rules.len() > 50 {
        return Err(ConfigError::ValidationError(format!(
            "consciousness.initial_rules: too many rules ({}), maximum is 50",
            config.consciousness.initial_rules.len()
        )));
    }
    for (i, rule) in config.consciousness.initial_rules.iter().enumerate() {
        if rule.trim().is_empty() {
            return Err(ConfigError::ValidationError(format!(
                "consciousness.initial_rules[{i}]: rule must not be empty or whitespace-only"
            )));
        }
    }

    // personality profile
    config
        .personality
        .validate()
        .map_err(|e| ConfigError::ValidationError(e.to_string()))?;

    Ok(())
}

/// Write a human-readable example config with comments for all options.
/// Used when creating a new config file on first run.
pub fn write_example_config() -> String {
    r#"# Archon CLI Configuration
# Generated on first run. Edit to customise your agent.
# Full path: ~/.config/archon/config.toml

[api]
# Model to use. Options: claude-sonnet-4-6, claude-opus-4-6, claude-haiku-4-5
default_model = "claude-sonnet-4-6"
thinking_budget = 16384
# effort: low, medium, high
default_effort = "medium"
max_retries = 3
# Override the API endpoint. Useful for LiteLLM, Ollama, or other proxies.
# Resolution order: ANTHROPIC_BASE_URL env var > this field > built-in default.
# base_url = "http://localhost:11434/v1/messages"

[identity]
# mode: "clean" (no spoofing) | "spoof" (mimic Claude Code headers) | "custom"
mode = "clean"
spoof_version = "2.1.89"
spoof_entrypoint = "cli"
anti_distillation = false

[personality]
# Your agent's name — used in the system prompt
name = "Archon"
# MBTI type — must be one of the 16 valid types:
#   INTJ INTP ENTJ ENTP INFJ INFP ENFJ ENFP
#   ISTJ ISFJ ESTJ ESFJ ISTP ISFP ESTP ESFP
type = "INTJ"
# Enneagram — format: [1-9]w[1-9]  (e.g. 4w5, 9w1, 7w8)
enneagram = "4w5"
# Traits that shape behaviour (free-form strings)
traits = ["strategic", "direct", "self-critical", "truth-over-comfort"]
# Communication style description (free-form)
communication_style = "terse"

[consciousness]
# Enable the inner voice subsystem
inner_voice = true
# Score decay rate applied each turn (0.98 = 2% decay per turn)
energy_decay_rate = 0.98
# Core behavioral rules seeded into the memory graph on startup.
# If non-empty, these replace the built-in defaults.
# Idempotent: rules already in the DB are never duplicated; new rules
# added here are picked up on the next startup automatically.
# Maximum 50 rules. Each must be a non-empty string.
initial_rules = [
    "Always ask before modifying files",
    "Explain reasoning before acting",
    "Never create files unless explicitly requested",
]

[tools]
# Maximum seconds a bash command may run before being killed
bash_timeout = 120
# Maximum bytes of bash output to capture
bash_max_output = 102400
# Maximum concurrent tool calls
max_concurrency = 4

[permissions]
# mode: default | acceptEdits | plan | auto | dontAsk | bypassPermissions
mode = "default"
allow_paths = []
deny_paths = []

[context]
# Compact conversation when context reaches this fraction of the limit (0.0–1.0)
compact_threshold = 0.80
preserve_recent_turns = 3
prompt_cache = true

[memory]
enabled = true

[cost]
# Warn when session cost exceeds this many USD (0.0 = disabled)
warn_threshold = 5.0
# Hard stop when session cost exceeds this many USD (0.0 = disabled)
hard_limit = 0.0

[logging]
level = "info"
max_files = 50
max_file_size_mb = 10

[session]
auto_resume = true

[checkpoint]
enabled = true
max_checkpoints = 10

[tui]
# Enable vim-style keybindings in the input field
vim_mode = false
"#
    .to_string()
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
        fs::write(&path, write_example_config())?;
        return Ok(ArchonConfig::default());
    }

    let content = fs::read_to_string(&path)?;
    let config: ArchonConfig = toml::from_str(&content)?;
    validate(&config)?;
    Ok(config)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_rules_default_is_empty() {
        assert!(ConsciousnessConfig::default().initial_rules.is_empty());
    }

    #[test]
    fn initial_rules_deserialized_from_toml() {
        let toml_str = r#"
            [consciousness]
            initial_rules = ["rule a", "rule b"]
        "#;
        let cfg: ArchonConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.consciousness.initial_rules.len(), 2);
        assert_eq!(cfg.consciousness.initial_rules[0], "rule a");
    }

    #[test]
    fn initial_rules_empty_string_rejected() {
        let mut cfg = ArchonConfig::default();
        cfg.consciousness.initial_rules = vec!["".to_string()];
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn initial_rules_whitespace_only_rejected() {
        let mut cfg = ArchonConfig::default();
        cfg.consciousness.initial_rules = vec!["   ".to_string()];
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("whitespace"));
    }

    #[test]
    fn initial_rules_max_50_enforced() {
        let mut cfg = ArchonConfig::default();
        cfg.consciousness.initial_rules = (0..51).map(|i| format!("rule {i}")).collect();
        assert!(validate(&cfg).is_err());

        cfg.consciousness.initial_rules = (0..50).map(|i| format!("rule {i}")).collect();
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn write_example_config_is_valid_toml() {
        let s = write_example_config();
        let cfg: ArchonConfig = toml::from_str(&s).expect("should parse as ArchonConfig");
        validate(&cfg).expect("should validate");
    }

    #[test]
    fn write_example_config_contains_personality_section() {
        assert!(write_example_config().contains("[personality]"));
    }

    #[test]
    fn write_example_config_contains_consciousness_section() {
        assert!(write_example_config().contains("[consciousness]"));
    }

    #[test]
    fn write_example_config_contains_initial_rules() {
        assert!(write_example_config().contains("initial_rules"));
    }

    #[test]
    fn write_example_config_personality_fields_round_trip() {
        let s = write_example_config();
        let cfg: ArchonConfig = toml::from_str(&s).unwrap();
        assert_eq!(cfg.personality.name, "Archon");
        assert_eq!(cfg.personality.mbti_type, "INTJ");
        assert_eq!(cfg.personality.enneagram, "4w5");
        assert!(!cfg.personality.traits.is_empty());
    }

    #[test]
    fn write_example_config_initial_rules_non_empty() {
        let s = write_example_config();
        let cfg: ArchonConfig = toml::from_str(&s).unwrap();
        assert!(!cfg.consciousness.initial_rules.is_empty());
    }
}
