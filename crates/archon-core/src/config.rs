use std::collections::{BTreeMap, HashMap};
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
    #[serde(default)]
    pub providers: ProvidersConfig,
    /// Provider-namespaced model aliases. Bumping a default model only
    /// requires editing one entry here; agent code uses aliases.
    #[serde(default)]
    pub models: ModelsConfig,
    pub identity: IdentityConfig,
    pub tools: ToolsConfig,
    pub permissions: PermissionsConfig,
    pub context: ContextConfig,
    pub memory: MemoryConfig,
    pub learning: LearningConfig,
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
    /// Sandbox backend configuration.
    #[serde(default)]
    pub sandbox: crate::sandbox::SandboxConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProvidersConfig {
    #[serde(rename = "openai-codex")]
    pub openai_codex: CodexProviderConfig,
}

// ---------------------------------------------------------------------------
// Models config — provider-namespaced model alias map.
// ---------------------------------------------------------------------------
//
// archon agent code refers to models by alias (e.g. `"sonnet"`, `"opus"`,
// `"haiku"` for Anthropic; `"default"`, `"codex"`, `"mini"` for Codex).
// At runtime the alias is resolved against this config; literal model
// identifiers pass through unchanged.
//
// Bumping a default Anthropic or Codex model requires editing exactly one
// entry here. Operators can override per-installation by setting these in
// `~/.config/archon/config.toml` or the project-local layer.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelsConfig {
    pub anthropic: AnthropicModelsConfig,
    #[serde(rename = "openai-codex")]
    pub openai_codex: OpenAiCodexModelsConfig,
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            anthropic: AnthropicModelsConfig::default(),
            openai_codex: OpenAiCodexModelsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AnthropicModelsConfig {
    pub opus: String,
    pub sonnet: String,
    pub haiku: String,
}

impl Default for AnthropicModelsConfig {
    fn default() -> Self {
        Self {
            opus: "claude-opus-4-7".into(),
            sonnet: "claude-sonnet-4-6".into(),
            haiku: "claude-haiku-4-5-20251001".into(),
        }
    }
}

impl AnthropicModelsConfig {
    /// Convert to the runtime alias map owned by `archon_llm::providers::AnthropicProvider`.
    ///
    /// This is the binary's job: read `config.models.anthropic`, call
    /// `to_alias_map()`, pass via `AnthropicProvider::with_alias_map(..)` so
    /// operator overrides reach the provider at construction.
    pub fn to_alias_map(&self) -> archon_llm::providers::anthropic::AnthropicAliasMap {
        archon_llm::providers::anthropic::AnthropicAliasMap {
            opus: self.opus.clone(),
            sonnet: self.sonnet.clone(),
            haiku: self.haiku.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenAiCodexModelsConfig {
    pub default: String,
    pub codex: String,
    pub mini: String,
}

impl Default for OpenAiCodexModelsConfig {
    fn default() -> Self {
        // Per OpenAI's Codex models reference (https://developers.openai.com/codex/models):
        // - default: gpt-5.5 is the newest/frontier model; gpt-5.4 is the
        //   documented fallback. Operators can override to gpt-5.4 here if
        //   they cannot reach 5.5 yet.
        // - codex: gpt-5.3-codex is the current codex-specific model for
        //   complex software engineering.
        // - mini: gpt-5.4-mini is the efficient/subagent variant.
        Self {
            default: "gpt-5.5".into(),
            codex: "gpt-5.3-codex".into(),
            mini: "gpt-5.4-mini".into(),
        }
    }
}

impl OpenAiCodexModelsConfig {
    /// Convert to the runtime alias map owned by `archon_llm::providers::CodexProvider`.
    ///
    /// Tier mapping for cross-provider neutrality:
    /// - `opus` tier (smartest) → `default` (frontier flagship)
    /// - `sonnet` tier (smart) → `default` (frontier flagship — same model
    ///   for now; can be split if Codex adds a smartest tier above gpt-5.5)
    /// - `haiku` tier (fast) → `mini`
    pub fn to_alias_map(&self) -> archon_llm::providers::codex::CodexAliasMap {
        archon_llm::providers::codex::CodexAliasMap {
            opus: self.default.clone(),
            sonnet: self.default.clone(),
            haiku: self.mini.clone(),
            codex: self.codex.clone(),
        }
    }
}

/// Resolve an Anthropic alias (or pass-through ID) using the provided
/// `[models.anthropic]` config slice.
///
/// Aliases recognised: `opus`, `sonnet`, `haiku` (case-insensitive). Anything
/// else is returned as-is so literal model IDs (e.g. `claude-sonnet-4-6`,
/// `claude-opus-4-7`) work without the resolver rejecting them.
pub fn resolve_anthropic_model(alias_or_id: &str, cfg: &AnthropicModelsConfig) -> String {
    match alias_or_id.trim().to_lowercase().as_str() {
        "opus" => cfg.opus.clone(),
        "sonnet" => cfg.sonnet.clone(),
        "haiku" => cfg.haiku.clone(),
        _ => alias_or_id.to_string(),
    }
}

/// Resolve a Codex alias (or pass-through ID) using the provided
/// `[models.openai-codex]` config slice.
///
/// Aliases recognised: `default`, `codex`, `mini` (case-insensitive). Anything
/// else is returned as-is. Empty input is treated as `default`.
pub fn resolve_codex_model(alias_or_id: &str, cfg: &OpenAiCodexModelsConfig) -> String {
    let lowered = alias_or_id.trim().to_lowercase();
    if lowered.is_empty() {
        return cfg.default.clone();
    }
    match lowered.as_str() {
        "default" => cfg.default.clone(),
        "codex" => cfg.codex.clone(),
        "mini" => cfg.mini.clone(),
        _ => alias_or_id.to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CodexProviderConfig {
    pub enabled: bool,
    pub runtime: String,
    pub direct_fallback: bool,
    pub app_server_transport: String,
    pub app_server_url: Option<String>,
    pub app_server_command: String,
    pub app_server_args: Vec<String>,
    pub app_server_discovery_timeout_ms: u64,
    pub app_server_model_catalog: Vec<String>,
    pub spoof: CodexSpoofPartialConfig,
    pub manifest: CodexManifestConfig,
}

impl Default for CodexProviderConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            runtime: "direct".into(),
            direct_fallback: false,
            app_server_transport: "websocket".into(),
            app_server_url: None,
            app_server_command: "codex".into(),
            app_server_args: vec!["app-server".into()],
            app_server_discovery_timeout_ms: 2_500,
            app_server_model_catalog: vec!["gpt-5.5".into(), "gpt-5.4".into()],
            spoof: CodexSpoofPartialConfig::default(),
            manifest: CodexManifestConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CodexSpoofPartialConfig {
    pub originator: Option<String>,
    pub user_agent: Option<String>,
    pub client_id: Option<String>,
    pub openai_beta: Option<String>,
    pub extra_headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CodexManifestConfig {
    pub fetch_url: String,
    pub ttl_seconds: u64,
    pub cache_dir: String,
}

impl Default for CodexManifestConfig {
    fn default() -> Self {
        Self {
            fetch_url: "https://raw.githubusercontent.com/ste-bah/archon-cli/main/crates/archon-llm/resources/codex-compat.json".into(),
            ttl_seconds: 21_600,
            cache_dir: "~/.archon/cache/codex-compat".into(),
        }
    }
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
    /// The active provider name (e.g. `"anthropic"`, `"openai-codex"`, `"openai"`, `"bedrock"`, `"vertex"`, `"local"`).
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
    /// Bedrock model ID (e.g. `"anthropic.claude-sonnet-4-6-v1:0"`).
    pub model_id: String,
}

impl Default for LlmBedrockConfig {
    fn default() -> Self {
        Self {
            region: "us-east-1".to_string(),
            model_id: "anthropic.claude-sonnet-4-6-v1:0".to_string(),
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
    /// Model name (e.g. `"claude-sonnet-4-6@20250514"`).
    pub model: String,
    /// Path to service account credentials JSON file.
    pub credentials_file: Option<String>,
}

impl Default for LlmVertexConfig {
    fn default() -> Self {
        Self {
            project_id: None,
            region: "us-central1".to_string(),
            model: "claude-sonnet-4-6@20250514".to_string(),
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
            anti_distillation: false,
            workload: None,
            custom: None,
        }
    }
}

impl IdentityConfig {
    pub fn as_view(&self) -> archon_llm::identity::IdentityConfigView<'_> {
        archon_llm::identity::IdentityConfigView {
            mode: &self.mode,
            spoof_version: &self.spoof_version,
            spoof_entrypoint: &self.spoof_entrypoint,
            spoof_betas: self.spoof_betas.as_deref(),
            anti_distillation: self.anti_distillation,
            workload: self.workload.as_deref(),
            custom: self.custom.as_ref().map(|custom| {
                archon_llm::identity::CustomIdentityConfigView {
                    user_agent: &custom.user_agent,
                    x_app: &custom.x_app,
                    extra_headers: custom.extra_headers.as_ref(),
                }
            }),
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
    pub preflight_safety_margin: f32,
    pub max_tokens: Option<u32>,
    pub context_window_override: Option<u64>,
    pub output_reserve_tokens: u64,
    pub preserve_recent_turns: u32,
    pub manual_compact_force_strategy: String,
    pub rate_limit_pressure_tokens: Option<u64>,
    pub rate_limit_pressure_body_bytes: Option<u64>,
    pub large_request_retry_body_bytes: Option<u64>,
    /// Whether to use prompt caching (cache_control breakpoints on static blocks).
    pub prompt_cache: bool,
    pub prompt_cache_mode: String,
    pub prompt_cache_ttl: String,
    pub prompt_cache_conversation: bool,
    /// Maximum characters for hierarchical ARCHON.md content.
    #[serde(alias = "claudemd_max_tokens")]
    pub archonmd_max_tokens: u32,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            compact_threshold: 0.80,
            preflight_safety_margin: 0.05,
            max_tokens: None,
            context_window_override: None,
            output_reserve_tokens: 8192,
            preserve_recent_turns: 3,
            manual_compact_force_strategy: "micro".into(),
            rate_limit_pressure_tokens: None,
            rate_limit_pressure_body_bytes: None,
            large_request_retry_body_bytes: None,
            prompt_cache: true,
            prompt_cache_mode: "explicit".into(),
            prompt_cache_ttl: "5m".into(),
            prompt_cache_conversation: false,
            archonmd_max_tokens: 8192,
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
    /// Memory garden consolidation settings.
    pub garden: archon_memory::garden::GardenConfig,
    /// Auto-capture settings (regex-based memory detection at turn boundary).
    pub auto_capture: AutoCaptureConfig,
    /// Auto-extraction settings (LLM-driven fact extraction every N turns).
    pub auto_extraction: AutoExtractionConfig,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            db_path: None,
            embedding_provider: archon_memory::embedding::EmbeddingProviderKind::Auto,
            hybrid_alpha: 0.3,
            garden: archon_memory::garden::GardenConfig::default(),
            auto_capture: AutoCaptureConfig::default(),
            auto_extraction: AutoExtractionConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoCaptureConfig {
    pub enabled: bool,
}

impl Default for AutoCaptureConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoExtractionConfig {
    pub enabled: bool,
    pub every_n_turns: u32,
}

impl Default for AutoExtractionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            every_n_turns: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct LearningConfig {
    pub sona: ToggleConfig,
    pub provenance: ToggleConfig,
    pub desc: ToggleConfig,
    pub gnn: GnnModelConfig,
    pub world_model: WorldModelConfig,
    pub reasoning_quality: ReasoningQualityConfig,
    pub session_briefing: SessionBriefingConfig,
    pub causal_memory: ToggleConfig,
    pub shadow_vector: ToggleConfig,
    pub reasoning_bank: ToggleConfig,
    pub reflexion: ReflexionConfig,
    pub agent_evolution: AgentEvolutionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasoningQualityConfig {
    pub enabled: bool,
    pub emit_inline_events: bool,
    pub post_turn_analysis: bool,
    pub post_session_analysis: bool,
    pub shadow_mode_days: u32,
    pub apply_trust_updates_after_shadow: bool,
    pub max_claims_per_turn: usize,
    pub max_excerpt_chars: usize,
    pub store_raw_text: bool,
    pub link_user_corrections: bool,
    pub update_self_trust: bool,
    pub feed_world_model: bool,
    pub feed_retrospective: bool,
    pub critic: ReasoningQualityCriticConfig,
    pub extractor_eval: ReasoningQualityExtractorEvalConfig,
    pub patterns: ReasoningQualityPatternsConfig,
}

impl Default for ReasoningQualityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            emit_inline_events: true,
            post_turn_analysis: true,
            post_session_analysis: true,
            shadow_mode_days: 30,
            apply_trust_updates_after_shadow: true,
            max_claims_per_turn: 12,
            max_excerpt_chars: 600,
            store_raw_text: false,
            link_user_corrections: true,
            update_self_trust: true,
            feed_world_model: true,
            feed_retrospective: true,
            critic: ReasoningQualityCriticConfig::default(),
            extractor_eval: ReasoningQualityExtractorEvalConfig::default(),
            patterns: ReasoningQualityPatternsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasoningQualityCriticConfig {
    pub mode: String,
    pub allow_llm: bool,
    pub provider: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub max_turns_per_session: usize,
    pub run_async: bool,
    pub fallback_to_heuristic: bool,
    pub budget: ReasoningQualityCriticBudgetConfig,
}

impl Default for ReasoningQualityCriticConfig {
    fn default() -> Self {
        Self {
            mode: "hybrid".to_string(),
            allow_llm: false,
            provider: "default".to_string(),
            model: String::new(),
            max_tokens: 1200,
            temperature: 0.0,
            max_turns_per_session: 50,
            run_async: true,
            fallback_to_heuristic: true,
            budget: ReasoningQualityCriticBudgetConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasoningQualityCriticBudgetConfig {
    pub per_session_token_cap: u64,
    pub daily_usd_cap: f64,
    pub weekly_usd_cap: f64,
    pub respect_provider_cooldowns: bool,
    pub emit_cost_events: bool,
}

impl Default for ReasoningQualityCriticBudgetConfig {
    fn default() -> Self {
        Self {
            per_session_token_cap: 200_000,
            daily_usd_cap: 10.0,
            weekly_usd_cap: 50.0,
            respect_provider_cooldowns: true,
            emit_cost_events: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasoningQualityExtractorEvalConfig {
    pub fixture_dir: String,
    pub min_claim_precision: f32,
    pub min_claim_recall: f32,
    pub min_claim_before_source_precision: f32,
    pub max_code_fence_false_positive_rate: f32,
    pub max_quoted_user_false_positive_rate: f32,
}

impl Default for ReasoningQualityExtractorEvalConfig {
    fn default() -> Self {
        Self {
            fixture_dir: "crates/archon-reasoning-quality/tests/fixtures/labeled_turns".to_string(),
            min_claim_precision: 0.85,
            min_claim_recall: 0.50,
            min_claim_before_source_precision: 0.90,
            max_code_fence_false_positive_rate: 0.05,
            max_quoted_user_false_positive_rate: 0.05,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasoningQualityPatternsConfig {
    pub window_days: u32,
    pub min_events: usize,
    pub min_distinct_sessions: usize,
    pub repeated_pattern_trust_weight: f32,
}

impl Default for ReasoningQualityPatternsConfig {
    fn default() -> Self {
        Self {
            window_days: 30,
            min_events: 3,
            min_distinct_sessions: 3,
            repeated_pattern_trust_weight: 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionBriefingConfig {
    pub enabled: bool,
    pub include_memory: bool,
    pub include_reasoning_quality: bool,
    pub include_pending_behaviour_proposals: bool,
    pub include_world_model: bool,
    pub max_items: usize,
    pub max_chars: usize,
    pub world_model_requires_ready: bool,
}

impl Default for SessionBriefingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            include_memory: true,
            include_reasoning_quality: true,
            include_pending_behaviour_proposals: true,
            include_world_model: true,
            max_items: 8,
            max_chars: 4000,
            world_model_requires_ready: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentEvolutionConfig {
    /// Governed Cozo profile versions exist by default, but runtime overlay is
    /// opt-in until enough shadow/e2e coverage proves the path for operators.
    pub active_profile_overlay_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToggleConfig {
    pub enabled: bool,
}

impl ToggleConfig {
    pub const fn enabled() -> Self {
        Self { enabled: true }
    }
}

impl Default for ToggleConfig {
    fn default() -> Self {
        Self::enabled()
    }
}

// ---------------------------------------------------------------------------
// GNN model configuration — [learning.gnn]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GnnModelConfig {
    pub enabled: bool,
    pub input_dim: usize,
    pub output_dim: usize,
    pub num_layers: usize,
    pub attention_heads: usize,
    pub max_nodes: usize,
    pub use_residual: bool,
    pub use_layer_norm: bool,
    pub activation: String,
    pub weight_seed: u64,
    #[serde(alias = "training")]
    pub training: GnnTrainingConfig,
    pub auto_trainer: GnnAutoTrainerConfig,
}

impl Default for GnnModelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            input_dim: 1536,
            output_dim: 1536,
            num_layers: 3,
            attention_heads: 12,
            max_nodes: 50,
            use_residual: true,
            use_layer_norm: true,
            activation: "relu".to_string(),
            weight_seed: 0,
            training: GnnTrainingConfig::default(),
            auto_trainer: GnnAutoTrainerConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// GNN training configuration — [learning.gnn.training]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GnnTrainingConfig {
    pub learning_rate: f32,
    pub batch_size: usize,
    pub max_epochs: usize,
    pub early_stopping_patience: usize,
    pub validation_split: f32,
    pub ewc_lambda: f32,
    pub margin: f32,
    pub triplet_loss_coefficient: f32,
    pub max_gradient_norm: f32,
    pub max_triplets_per_run: usize,
    pub max_runtime_ms: u64,
}

impl Default for GnnTrainingConfig {
    fn default() -> Self {
        Self {
            learning_rate: 0.001,
            batch_size: 32,
            max_epochs: 10,
            early_stopping_patience: 3,
            validation_split: 0.2,
            ewc_lambda: 0.1,
            margin: 0.5,
            triplet_loss_coefficient: 0.1,
            max_gradient_norm: 1.0,
            max_triplets_per_run: 256,
            max_runtime_ms: 300_000,
        }
    }
}

// ---------------------------------------------------------------------------
// GNN auto-trainer configuration — [learning.gnn.auto_trainer]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GnnAutoTrainerConfig {
    pub enabled: bool,
    /// Minimum time between training runs in ms (throttle).
    pub min_throttle_ms: u64,
    /// Trigger training after N new memories since last train.
    pub trigger_new_memories: u64,
    /// Trigger training after this many ms since last train.
    pub trigger_elapsed_ms: u64,
    /// Trigger training after N corrections since last train.
    pub trigger_corrections: u64,
    /// Memories needed before the first training run.
    pub first_run_threshold: u64,
    /// Max wall-clock time per training run in ms.
    pub max_runtime_ms: u64,
    /// Background tick interval in ms.
    pub tick_interval_ms: u64,
}

impl Default for GnnAutoTrainerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_throttle_ms: 3_600_000,
            trigger_new_memories: 20,
            trigger_elapsed_ms: 21_600_000,
            trigger_corrections: 3,
            first_run_threshold: 30,
            max_runtime_ms: 300_000,
            tick_interval_ms: 60_000,
        }
    }
}

// ---------------------------------------------------------------------------
// Local world model configuration — [learning.world_model]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelConfig {
    pub enabled: bool,
    pub model_kind: String,
    pub auto_promote_advisory: bool,
    pub require_approval_for_behavior_change: bool,
    pub state_dim: usize,
    pub max_checkpoint_mb: u64,
    pub max_prediction_latency_ms: u64,
    pub max_counterfactual_actions: usize,
    pub store_raw_text: bool,
    pub include_conversation_turns: bool,
    pub include_agent_outputs: bool,
    pub embeddings: WorldModelEmbeddingsConfig,
    pub labeler: WorldModelLabelerConfig,
    pub training: WorldModelTrainingConfig,
    pub jepa: WorldModelJepaConfig,
    pub eval: WorldModelEvalConfig,
    pub cold_start: WorldModelColdStartConfig,
    pub auto_trainer: WorldModelAutoTrainerConfig,
    pub guardrails: WorldModelGuardrailsConfig,
    pub retention: WorldModelRetentionConfig,
}

impl Default for WorldModelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model_kind: "latent_transition".into(),
            auto_promote_advisory: true,
            require_approval_for_behavior_change: true,
            state_dim: 384,
            max_checkpoint_mb: 64,
            max_prediction_latency_ms: 100,
            max_counterfactual_actions: 5,
            store_raw_text: false,
            include_conversation_turns: true,
            include_agent_outputs: true,
            embeddings: WorldModelEmbeddingsConfig::default(),
            labeler: WorldModelLabelerConfig::default(),
            training: WorldModelTrainingConfig::default(),
            jepa: WorldModelJepaConfig::default(),
            eval: WorldModelEvalConfig::default(),
            cold_start: WorldModelColdStartConfig::default(),
            auto_trainer: WorldModelAutoTrainerConfig::default(),
            guardrails: WorldModelGuardrailsConfig::default(),
            retention: WorldModelRetentionConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelEmbeddingsConfig {
    pub source: String,
    pub provider: String,
    pub model: String,
    pub dimensions: usize,
    pub projection_dim: usize,
    pub cache_enabled: bool,
    pub cache_max_mb: u64,
    pub redact_before_embedding: bool,
    pub allow_third_party: bool,
    pub external_base_url: String,
    pub external_api_key_env: String,
}

impl Default for WorldModelEmbeddingsConfig {
    fn default() -> Self {
        Self {
            source: "local".into(),
            provider: "fastembed".into(),
            model: "bge-base-en-v1.5".into(),
            dimensions: 768,
            projection_dim: 384,
            cache_enabled: true,
            cache_max_mb: 1_024,
            redact_before_embedding: true,
            allow_third_party: false,
            external_base_url: String::new(),
            external_api_key_env: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelLabelerConfig {
    pub analyzer: String,
    pub llm_enabled: bool,
    pub max_events_per_prompt: usize,
    pub max_prompt_chars: usize,
}

impl Default for WorldModelLabelerConfig {
    fn default() -> Self {
        Self {
            analyzer: "hybrid".into(),
            llm_enabled: true,
            max_events_per_prompt: 30,
            max_prompt_chars: 128_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelTrainingConfig {
    pub backend: String,
    pub allow_cpu_fallback: bool,
    pub prefer_accelerator: bool,
    pub precision: String,
    pub max_accelerator_memory_mb: u64,
    pub batch_size: usize,
    pub max_epochs: usize,
    pub validation_split: f32,
    pub promotion_min_delta: f32,
    pub max_runtime_ms: u64,
}

impl Default for WorldModelTrainingConfig {
    fn default() -> Self {
        Self {
            backend: "auto".into(),
            allow_cpu_fallback: true,
            prefer_accelerator: true,
            precision: "fp32".into(),
            max_accelerator_memory_mb: 4_096,
            batch_size: 32,
            max_epochs: 10,
            validation_split: 0.2,
            promotion_min_delta: 0.02,
            max_runtime_ms: 300_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelJepaConfig {
    pub enabled: bool,
    pub latent_dim: usize,
    pub context_window_rows: usize,
    pub target_window_rows: usize,
    pub prediction_horizons: Vec<usize>,
    pub mask_ratio: f32,
    pub ema_decay: f32,
    pub latent_var_floor: f32,
    pub min_latent_std: f32,
    pub min_effective_rank_ratio: f32,
    pub batch_size: usize,
    pub max_epochs: usize,
    pub learning_rate: f32,
    pub alpha_mse: f32,
    pub beta_aux: f32,
    pub gamma_horizon: f32,
    pub delta_var: f32,
    pub allow_generic_fallback: bool,
    pub max_runtime_ms: u64,
    pub max_prediction_latency_ms: u64,
    pub max_checkpoint_mb: u64,
    pub horizon_consistency_tol: f32,
    pub min_baseline_improvement: f32,
    pub min_heldout_examples: usize,
    pub min_training_examples: usize,
    pub require_native_accelerator_ops: bool,
    pub allow_accelerated_candidate_cpu_stage: bool,
    pub min_cuda_validation_examples: usize,
    pub min_metal_validation_examples: usize,
    pub backend_parity_cosine_floor: f32,
    pub max_backend_prediction_latency_ms: u64,
    pub max_backend_first_call_latency_ms: u64,
    /// Eval pipeline configuration (`[learning.world_model.jepa.eval]`). PRD-006C §12.
    #[serde(default)]
    pub eval: WorldModelJepaEvalConfig,
}

impl Default for WorldModelJepaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            latent_dim: 384,
            context_window_rows: 8,
            target_window_rows: 3,
            prediction_horizons: vec![1, 3, 5],
            mask_ratio: 0.30,
            ema_decay: 0.996,
            latent_var_floor: 0.05,
            min_latent_std: 0.05,
            min_effective_rank_ratio: 0.50,
            batch_size: 32,
            max_epochs: 10,
            learning_rate: 0.001,
            alpha_mse: 0.25,
            beta_aux: 0.50,
            gamma_horizon: 0.10,
            delta_var: 0.10,
            allow_generic_fallback: true,
            max_runtime_ms: 300_000,
            max_prediction_latency_ms: 50,
            max_checkpoint_mb: 64,
            horizon_consistency_tol: 0.02,
            min_baseline_improvement: 0.05,
            min_heldout_examples: 200,
            min_training_examples: 2_000,
            require_native_accelerator_ops: true,
            allow_accelerated_candidate_cpu_stage: false,
            min_cuda_validation_examples: 512,
            min_metal_validation_examples: 512,
            backend_parity_cosine_floor: 0.99,
            max_backend_prediction_latency_ms: 50,
            max_backend_first_call_latency_ms: 5_000,
            eval: WorldModelJepaEvalConfig::default(),
        }
    }
}

impl WorldModelJepaConfig {
    /// Returns the eval_schema_version used for config_fingerprint and cache_key
    /// computations.
    ///
    /// T025: reads from the nested `eval` sub-config (no longer hardcoded 1u32).
    /// Bump `learning.world_model.jepa.eval.eval_schema_version` in config to
    /// invalidate all existing embedding cache entries.
    pub fn eval_schema_version_or_default(&self) -> u32 {
        // T025: now reads from nested eval config, not hardcoded
        self.eval.eval_schema_version
    }
}

/// Eval pipeline configuration for `[learning.world_model.jepa.eval]`.
/// PRD-006C §12.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelJepaEvalConfig {
    /// Default eval mode: `"quick"` | `"full"` | `"promotion"`
    pub mode: String,
    /// JEPA encoding batch size (transitions per batch)
    pub batch_size: usize,
    /// Fastembed baseline embedding batch size
    pub embedding_batch_size: usize,
    /// Progress output interval in rows
    pub progress_interval_rows: usize,
    /// Quick-mode runtime budget in ms (must be > 0; quick must be fast)
    pub quick_max_runtime_ms: u64,
    /// Full-mode runtime budget in ms (0 = unlimited; handles 30+ min workloads)
    pub full_max_runtime_ms: u64,
    /// Promotion-mode runtime budget in ms (0 = unlimited)
    pub promotion_max_runtime_ms: u64,
    /// Heartbeat interval for stale lock detection when budget = 0
    pub stale_heartbeat_ms: u64,
    /// Enable embedding cache reads and writes
    pub cache_enabled: bool,
    /// Maximum embedding cache size in MB (LRU eviction when exceeded)
    pub cache_max_mb: u64,
    /// Default to background execution (requires `policy.allow_eval_background_jobs`)
    pub background_default: bool,
    /// Schema version for embedding cache key invalidation.
    /// Bump this value to invalidate ALL existing cache entries.
    pub eval_schema_version: u32,
}

impl Default for WorldModelJepaEvalConfig {
    fn default() -> Self {
        Self {
            mode: "quick".to_string(),
            batch_size: 256,
            embedding_batch_size: 64,
            progress_interval_rows: 500,
            quick_max_runtime_ms: 30_000,
            full_max_runtime_ms: 0,
            promotion_max_runtime_ms: 0,
            stale_heartbeat_ms: 120_000,
            cache_enabled: true,
            cache_max_mb: 2048,
            background_default: false,
            eval_schema_version: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelEvalConfig {
    pub bootstrap_iterations: usize,
    pub confidence_level: f32,
    pub parity_precision: String,
    pub parity_min_cosine: f32,
    pub next_state_baseline_min_delta: f32,
    pub counterfactual_baseline_min_delta: f32,
    pub surprise_ks_min_p: f32,
    pub counterfactual_ndcg_min: f32,
}

impl Default for WorldModelEvalConfig {
    fn default() -> Self {
        Self {
            bootstrap_iterations: 1_000,
            confidence_level: 0.95,
            parity_precision: "fp32".into(),
            parity_min_cosine: 0.95,
            next_state_baseline_min_delta: 0.10,
            counterfactual_baseline_min_delta: 0.10,
            surprise_ks_min_p: 0.05,
            counterfactual_ndcg_min: 0.60,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelColdStartConfig {
    pub min_rows: u64,
    pub min_sessions: u64,
    pub min_observed_days: u64,
}

impl Default for WorldModelColdStartConfig {
    fn default() -> Self {
        Self {
            min_rows: 1_000,
            min_sessions: 50,
            min_observed_days: 7,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelAutoTrainerConfig {
    pub enabled: bool,
    pub min_throttle_ms: u64,
    pub idle_required_ms: u64,
    pub battery_suspend_below_percent: u8,
    pub trigger_new_rows: u64,
    pub trigger_surprises: u64,
    pub trigger_corrections: u64,
    pub trigger_elapsed_ms: u64,
    pub first_run_threshold: u64,
    pub max_runtime_ms: u64,
    pub tick_interval_ms: u64,
}

impl Default for WorldModelAutoTrainerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_throttle_ms: 3_600_000,
            idle_required_ms: 300_000,
            battery_suspend_below_percent: 30,
            trigger_new_rows: 100,
            trigger_surprises: 5,
            trigger_corrections: 3,
            trigger_elapsed_ms: 21_600_000,
            first_run_threshold: 300,
            max_runtime_ms: 300_000,
            tick_interval_ms: 60_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelGuardrailsConfig {
    pub enabled: bool,
    pub interactive_mode: String,
    pub pipeline_mode: String,
    pub tool_run_mode: String,
    pub verification_run_mode: String,
    pub high_risk_threshold: f32,
    pub medium_risk_threshold: f32,
    pub critical_risk_threshold: f32,
    pub require_tests_for_coding_high_risk: bool,
    pub require_build_for_coding_high_risk: bool,
    pub require_lint_for_coding_high_risk: bool,
    pub require_typecheck_for_coding_high_risk: bool,
    pub require_plan_review_for_plan_drift: bool,
    pub require_source_check_for_research_high_risk: bool,
    pub require_manual_approval_for_critical: bool,
    pub max_guardrail_overhead_ms: u64,
    pub record_outcomes_without_prediction: bool,
    pub max_guardrail_events_per_session: usize,
}

impl Default for WorldModelGuardrailsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interactive_mode: "advisory".into(),
            pipeline_mode: "guarded".into(),
            tool_run_mode: "learn_only".into(),
            verification_run_mode: "learn_only".into(),
            high_risk_threshold: 0.70,
            medium_risk_threshold: 0.45,
            critical_risk_threshold: 0.85,
            require_tests_for_coding_high_risk: true,
            require_build_for_coding_high_risk: true,
            require_lint_for_coding_high_risk: false,
            require_typecheck_for_coding_high_risk: false,
            require_plan_review_for_plan_drift: true,
            require_source_check_for_research_high_risk: true,
            require_manual_approval_for_critical: false,
            max_guardrail_overhead_ms: 40,
            record_outcomes_without_prediction: true,
            max_guardrail_events_per_session: 500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelRetentionConfig {
    pub jsonl_rotate_mb: u64,
    pub raw_retention_days: u64,
    pub retain_cozo_summaries: bool,
    pub retain_checkpoint_count: usize,
}

impl Default for WorldModelRetentionConfig {
    fn default() -> Self {
        Self {
            jsonl_rotate_mb: 500,
            raw_retention_days: 90,
            retain_cozo_summaries: true,
            retain_checkpoint_count: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReflexionConfig {
    pub enabled: bool,
    pub max_per_agent: usize,
}

impl Default for ReflexionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_per_agent: 3,
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
    /// Energy regenerated after successful tool use.
    pub energy_regen_rate: f32,
    /// Minimum active-session energy.
    pub energy_floor: f32,
    /// Behavioral rules to seed into the memory graph on startup.
    /// If non-empty, these replace the built-in defaults.
    /// Idempotent: rules already present are not duplicated.
    /// Maximum 50 rules. Each must be a non-empty string.
    pub initial_rules: Vec<String>,
    /// Whether to persist personality state (InnerVoice + rule scores) across sessions.
    pub persist_personality: bool,
    /// Maximum number of personality snapshots to retain (oldest pruned first).
    pub personality_history_limit: u32,
}

impl Default for ConsciousnessConfig {
    fn default() -> Self {
        Self {
            inner_voice: true,
            energy_decay_rate: 0.98,
            energy_regen_rate: 0.005,
            energy_floor: 0.1,
            initial_rules: Vec::new(),
            persist_personality: true,
            personality_history_limit: 50,
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

    config
        .sandbox
        .validate()
        .map_err(ConfigError::ValidationError)?;

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
    if !(0.0..=1.0).contains(&config.consciousness.energy_regen_rate) {
        return Err(ConfigError::ValidationError(format!(
            "consciousness.energy_regen_rate must be 0.0..=1.0, got {}",
            config.consciousness.energy_regen_rate
        )));
    }
    if !(0.0..=1.0).contains(&config.consciousness.energy_floor) {
        return Err(ConfigError::ValidationError(format!(
            "consciousness.energy_floor must be 0.0..=1.0, got {}",
            config.consciousness.energy_floor
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

    validate_world_model_guardrails(&config.learning.world_model.guardrails)?;
    validate_world_model_jepa(&config.learning.world_model.jepa)?;

    // personality profile
    config
        .personality
        .validate()
        .map_err(|e| ConfigError::ValidationError(e.to_string()))?;

    Ok(())
}

fn validate_world_model_jepa(jepa: &WorldModelJepaConfig) -> Result<(), ConfigError> {
    if jepa.min_cuda_validation_examples == 0 || jepa.min_metal_validation_examples == 0 {
        return Err(ConfigError::ValidationError(
            "learning.world_model.jepa min_*_validation_examples must be > 0".into(),
        ));
    }
    if !(0.0..=1.0).contains(&jepa.backend_parity_cosine_floor) {
        return Err(ConfigError::ValidationError(format!(
            "learning.world_model.jepa.backend_parity_cosine_floor must be 0.0..=1.0, got {}",
            jepa.backend_parity_cosine_floor
        )));
    }
    if jepa.max_backend_prediction_latency_ms == 0 {
        return Err(ConfigError::ValidationError(
            "learning.world_model.jepa.max_backend_prediction_latency_ms must be > 0".into(),
        ));
    }
    if jepa.max_backend_first_call_latency_ms == 0 {
        return Err(ConfigError::ValidationError(
            "learning.world_model.jepa.max_backend_first_call_latency_ms must be > 0".into(),
        ));
    }

    // T025: validate eval sub-config
    let eval = &jepa.eval;
    if !["quick", "full", "promotion"].contains(&eval.mode.as_str()) {
        return Err(ConfigError::ValidationError(
            "learning.world_model.jepa.eval.mode must be one of: quick, full, promotion".into(),
        ));
    }
    if eval.quick_max_runtime_ms == 0 {
        return Err(ConfigError::ValidationError(
            "learning.world_model.jepa.eval.quick_max_runtime_ms must be > 0 \
             (quick mode requires a bounded deadline)"
                .into(),
        ));
    }
    if eval.embedding_batch_size > eval.batch_size {
        return Err(ConfigError::ValidationError(format!(
            "learning.world_model.jepa.eval.embedding_batch_size ({}) must be <= batch_size ({})",
            eval.embedding_batch_size, eval.batch_size
        )));
    }
    if eval.eval_schema_version == 0 {
        return Err(ConfigError::ValidationError(
            "learning.world_model.jepa.eval.eval_schema_version must be >= 1".into(),
        ));
    }

    Ok(())
}

fn validate_world_model_guardrails(
    guardrails: &WorldModelGuardrailsConfig,
) -> Result<(), ConfigError> {
    for (name, value) in [
        ("interactive_mode", guardrails.interactive_mode.as_str()),
        ("pipeline_mode", guardrails.pipeline_mode.as_str()),
        ("tool_run_mode", guardrails.tool_run_mode.as_str()),
        (
            "verification_run_mode",
            guardrails.verification_run_mode.as_str(),
        ),
    ] {
        if !matches!(
            value,
            "off" | "learn_only" | "advisory" | "guarded" | "strict"
        ) {
            return Err(ConfigError::ValidationError(format!(
                "learning.world_model.guardrails.{name} must be off, learn_only, advisory, guarded, or strict, got \"{value}\""
            )));
        }
    }
    for (name, value) in [
        ("medium_risk_threshold", guardrails.medium_risk_threshold),
        ("high_risk_threshold", guardrails.high_risk_threshold),
        (
            "critical_risk_threshold",
            guardrails.critical_risk_threshold,
        ),
    ] {
        if !(0.0..=1.0).contains(&value) {
            return Err(ConfigError::ValidationError(format!(
                "learning.world_model.guardrails.{name} must be 0.0..=1.0, got {value}"
            )));
        }
    }
    if guardrails.medium_risk_threshold > guardrails.high_risk_threshold
        || guardrails.high_risk_threshold > guardrails.critical_risk_threshold
    {
        return Err(ConfigError::ValidationError(
            "learning.world_model.guardrails thresholds must satisfy medium <= high <= critical"
                .into(),
        ));
    }
    if guardrails.max_guardrail_overhead_ms == 0 {
        return Err(ConfigError::ValidationError(
            "learning.world_model.guardrails.max_guardrail_overhead_ms must be > 0".into(),
        ));
    }
    Ok(())
}

/// Write a human-readable example config with comments for all options.
/// Used when creating a new config file on first run.
pub fn write_example_config() -> String {
    include_str!("../../../config.toml").to_string()
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

/// Load configuration from an existing path without creating a default file.
pub fn load_config_if_exists(path: PathBuf) -> Result<Option<ArchonConfig>, ConfigError> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path)?;
    let config: ArchonConfig = toml::from_str(&content)?;
    validate(&config)?;
    Ok(Some(config))
}

/// GHOST-008: persist `voice.enabled` to the HOME config file.
///
/// Loads the existing config (or default if file missing), updates
/// `voice.enabled`, serializes back to TOML, and writes to
/// `~/.config/archon/config.toml`. Uses full-rewrite (not surgical
/// TOML edit) — the config file is machine-generated from defaults
/// and does not carry hand-curated comments worth preserving.
pub fn save_voice_enabled(enabled: bool) -> Result<(), ConfigError> {
    let path = default_config_path();
    let mut config = if path.exists() {
        let content = fs::read_to_string(&path)?;
        toml::from_str::<ArchonConfig>(&content)?
    } else {
        ArchonConfig::default()
    };
    config.voice.enabled = enabled;
    let serialized = toml::to_string_pretty(&config)
        .map_err(|e| ConfigError::ValidationError(format!("TOML serialize error: {e}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serialized)?;
    Ok(())
}

/// Persist selected world-model guardrail modes to the HOME config file.
pub fn save_world_model_guardrail_modes(
    interactive_mode: Option<&str>,
    pipeline_mode: Option<&str>,
) -> Result<PathBuf, ConfigError> {
    let path = default_config_path();
    let mut config = if path.exists() {
        let content = fs::read_to_string(&path)?;
        toml::from_str::<ArchonConfig>(&content)?
    } else {
        ArchonConfig::default()
    };
    if let Some(mode) = interactive_mode {
        config.learning.world_model.guardrails.interactive_mode = mode.to_string();
    }
    if let Some(mode) = pipeline_mode {
        config.learning.world_model.guardrails.pipeline_mode = mode.to_string();
    }
    validate(&config)?;
    let serialized = toml::to_string_pretty(&config)
        .map_err(|e| ConfigError::ValidationError(format!("TOML serialize error: {e}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serialized)?;
    Ok(path)
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
    fn write_example_config_contains_world_model_guardrails_section() {
        assert!(write_example_config().contains("[learning.world_model.guardrails]"));
        let cfg: ArchonConfig = toml::from_str(&write_example_config()).unwrap();
        assert_eq!(
            cfg.learning.world_model.guardrails.interactive_mode,
            "advisory"
        );
        assert_eq!(cfg.learning.world_model.guardrails.pipeline_mode, "guarded");
        assert_eq!(
            cfg.learning
                .world_model
                .guardrails
                .max_guardrail_overhead_ms,
            40
        );
    }

    #[test]
    fn world_model_guardrail_config_validation_rejects_bad_modes_and_thresholds() {
        let mut cfg = ArchonConfig::default();
        cfg.learning.world_model.guardrails.interactive_mode = "YOLO".into();
        assert!(validate(&cfg).is_err());

        let mut cfg = ArchonConfig::default();
        cfg.learning.world_model.guardrails.medium_risk_threshold = 0.80;
        cfg.learning.world_model.guardrails.high_risk_threshold = 0.70;
        assert!(validate(&cfg).is_err());

        let mut cfg = ArchonConfig::default();
        cfg.learning
            .world_model
            .guardrails
            .max_guardrail_overhead_ms = 0;
        assert!(validate(&cfg).is_err());
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

    #[test]
    fn ssh_agent_forwarding_defaults_to_false() {
        let cfg = ArchonConfig::default();
        assert!(!cfg.remote.ssh.agent_forwarding);
    }

    #[test]
    fn ssh_agent_forwarding_true_deserialized() {
        let toml_str = r#"
            [remote.ssh]
            agent_forwarding = true
        "#;
        let cfg: ArchonConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.remote.ssh.agent_forwarding);
    }

    #[test]
    fn ssh_agent_forwarding_false_deserialized() {
        let toml_str = r#"
            [remote.ssh]
            agent_forwarding = false
        "#;
        let cfg: ArchonConfig = toml::from_str(toml_str).unwrap();
        assert!(!cfg.remote.ssh.agent_forwarding);
    }

    #[test]
    fn ssh_agent_forwarding_absent_defaults_false() {
        let toml_str = r#"
            [remote.ssh]
            port = 2222
        "#;
        let cfg: ArchonConfig = toml::from_str(toml_str).unwrap();
        assert!(!cfg.remote.ssh.agent_forwarding);
    }

    // -------------------------------------------------------------------------
    // T025: WorldModelJepaEvalConfig validation tests
    // -------------------------------------------------------------------------

    #[test]
    fn validate_jepa_eval_rejects_invalid_mode() {
        let mut config = WorldModelJepaConfig::default();
        config.eval.mode = "invalid".to_string();
        let result = validate_world_model_jepa(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("mode"));
    }

    #[test]
    fn validate_jepa_eval_accepts_quick_full_promotion() {
        for valid_mode in &["quick", "full", "promotion"] {
            let mut config = WorldModelJepaConfig::default();
            config.eval.mode = valid_mode.to_string();
            assert!(
                validate_world_model_jepa(&config).is_ok(),
                "{valid_mode} must be valid"
            );
        }
    }

    #[test]
    fn validate_jepa_eval_rejects_zero_quick_runtime() {
        let mut config = WorldModelJepaConfig::default();
        config.eval.quick_max_runtime_ms = 0;
        let result = validate_world_model_jepa(&config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("quick_max_runtime_ms")
        );
    }

    #[test]
    fn validate_jepa_eval_rejects_oversized_embedding_batch() {
        let mut config = WorldModelJepaConfig::default();
        config.eval.batch_size = 64;
        config.eval.embedding_batch_size = 256; // > batch_size
        let result = validate_world_model_jepa(&config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("embedding_batch_size")
        );
    }

    #[test]
    fn validate_jepa_eval_rejects_zero_schema_version() {
        let mut config = WorldModelJepaConfig::default();
        config.eval.eval_schema_version = 0;
        let result = validate_world_model_jepa(&config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("eval_schema_version")
        );
    }

    #[test]
    fn default_jepa_eval_config_passes_validation() {
        let config = WorldModelJepaConfig::default();
        assert!(validate_world_model_jepa(&config).is_ok());
    }
}
