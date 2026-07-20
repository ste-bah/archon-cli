use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
#[derive(Default)]
pub struct ModelsConfig {
    pub anthropic: AnthropicModelsConfig,
    #[serde(rename = "openai-codex")]
    pub openai_codex: OpenAiCodexModelsConfig,
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
            opus: "claude-opus-4-8".into(),
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
/// `claude-opus-4-8`) work without the resolver rejecting them.
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
