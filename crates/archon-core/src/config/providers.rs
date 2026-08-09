use std::collections::BTreeMap;
use std::sync::LazyLock;

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
// Bumping a default Anthropic or Codex model is a one-line edit to
// `[models.*]` in the workspace-root `config.toml`; the `Default` impls below
// read that file, so no Rust change is needed. Operators override
// per-installation in `~/.config/archon/config.toml` or the project-local layer.

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

/// The shipped `config.toml`, parsed once as an untyped value tree.
///
/// This is the single source of truth for default model ids. `write_example_config()`
/// hands operators the same file, so the template and the compiled-in defaults
/// cannot drift — previously they did: the template said `claude-opus-5` and
/// `gpt-5.6-sol` while these impls still said `claude-opus-4-8` and `gpt-5.5`,
/// so any installation that omitted a `[models.*]` key silently got a stale model.
///
/// Parsed as `toml::Value`, deliberately *not* as `ArchonConfig`: the config
/// structs are `#[serde(default)]`, so deserialising into them would re-enter the
/// very `Default` impls defined below. An untyped tree has no serde involvement
/// and therefore no cycle.
static SHIPPED_TEMPLATE: LazyLock<toml::Value> = LazyLock::new(|| {
    include_str!("../../../../config.toml")
        .parse::<toml::Value>()
        .expect("shipped config.toml must be valid TOML")
});

/// Read `models.<provider>.<key>` from the shipped template.
///
/// Panics if the key is absent. The template is embedded at compile time, so a
/// missing key is a build-artifact defect that either always fires or never
/// does — `template_defaults_cover_every_model_key` pins every key this function
/// is called with, so a gap fails CI rather than reaching an operator.
fn template_model(provider: &str, key: &str) -> String {
    SHIPPED_TEMPLATE
        .get("models")
        .and_then(|models| models.get(provider))
        .and_then(|slice| slice.get(key))
        .and_then(toml::Value::as_str)
        .unwrap_or_else(|| panic!("shipped config.toml is missing models.{provider}.{key}"))
        .to_string()
}

/// Read a string array at `providers.<provider>.<key>` from the shipped template.
///
/// Same contract as `template_model`: absent or non-string entries are a
/// build-artifact defect, pinned by the drift suite rather than tolerated.
fn template_provider_list(provider: &str, key: &str) -> Vec<String> {
    SHIPPED_TEMPLATE
        .get("providers")
        .and_then(|providers| providers.get(provider))
        .and_then(|slice| slice.get(key))
        .and_then(toml::Value::as_array)
        .unwrap_or_else(|| panic!("shipped config.toml is missing providers.{provider}.{key}"))
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .unwrap_or_else(|| panic!("providers.{provider}.{key} must contain only strings"))
                .to_string()
        })
        .collect()
}

impl Default for AnthropicModelsConfig {
    fn default() -> Self {
        Self {
            opus: template_model("anthropic", "opus"),
            sonnet: template_model("anthropic", "sonnet"),
            haiku: template_model("anthropic", "haiku"),
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
        // Sourced from `[models.openai-codex]` in the shipped config.toml, which
        // carries the rationale for each choice. Bumping a default Codex model is
        // a template edit; no Rust change is needed.
        Self {
            default: template_model("openai-codex", "default"),
            codex: template_model("openai-codex", "codex"),
            mini: template_model("openai-codex", "mini"),
        }
    }
}

impl OpenAiCodexModelsConfig {
    /// Convert to the runtime alias map owned by `archon_llm::providers::CodexProvider`.
    ///
    /// Tier mapping for cross-provider neutrality:
    /// - `opus` tier (smartest) → `default` (frontier flagship)
    /// - `sonnet` tier (smart) → `default` (frontier flagship — same model
    ///   for now; can be split if Codex adds a tier above the flagship)
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
            // Sourced from the template, which lists the full catalog. The
            // hardcoded pair this replaced had gone stale: it offered only
            // gpt-5.5/gpt-5.4, so an operator who omitted the key got an
            // app-server catalog containing none of the 5.6 models.
            app_server_model_catalog: template_provider_list(
                "openai-codex",
                "app_server_model_catalog",
            ),
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
