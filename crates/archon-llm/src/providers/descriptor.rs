//! `ProviderDescriptor` — static registry entry describing one LLM provider.
//!
//! TECH-AGS-PROVIDERS lines 1063-1074. Pure data: the descriptor *names* the
//! environment variable holding the API key (NFR-SECURITY-001) but never
//! carries the secret itself. `OpenAiCompatProvider` (TASK-AGS-703) pairs a
//! descriptor with a `SecretString` and an `http_client` at runtime.

use std::collections::HashMap;

use url::Url;

use super::features::ProviderFeatures;
use super::quirks::ProviderQuirks;

/// Which HTTP authentication scheme a provider expects.
///
/// `Custom(String)` carries the header name (e.g. `"x-api-token"`) for
/// providers that deviate from `Authorization: Bearer <key>`. A plain
/// `String` (rather than `&'static str`) is required so the whole
/// descriptor can derive `serde::Deserialize` without lifetime escapes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "header", rename_all = "snake_case")]
pub enum AuthFlavor {
    /// `Authorization: Bearer <api_key>`. The common case for OpenAI-style
    /// providers.
    BearerApiKey,
    /// No authentication (e.g. a localhost Ollama instance).
    None,
    /// HTTP Basic Auth with the API key as the username.
    BasicAuth,
    /// Non-standard header name, e.g. `"x-api-token"` or `"api-key"`.
    Custom(String),
}

/// Whether the provider speaks the OpenAI `/v1/chat/completions` wire format
/// (so `OpenAiCompatProvider` can drive it) or requires a bespoke impl.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatKind {
    /// OpenAI-compatible chat-completions wire format.
    OpenAiCompat,
    /// Bespoke protocol; needs its own `LlmProvider` impl (Anthropic, Bedrock,
    /// Vertex, ...).
    Native,
}

/// Static registry entry for one provider.
///
/// The descriptor is the single source of truth for routing. `id` is the slug
/// used in config (`provider = "groq"`), `base_url` is the default endpoint,
/// and `env_key_var` names the environment variable whose value is the API
/// key. The secret value itself lives only in `OpenAiCompatProvider` /
/// `SecretString` at runtime.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderDescriptor {
    /// Config slug, e.g. `"ollama"`, `"groq"`, `"openrouter"`.
    pub id: String,
    /// Human-readable name, e.g. `"Groq"`.
    pub display_name: String,
    /// Default endpoint URL. Users may override at the `[llm.<provider>]`
    /// level; this field is the fallback.
    pub base_url: Url,
    /// HTTP auth scheme.
    pub auth_flavor: AuthFlavor,
    /// Environment variable that holds the API key, e.g. `"GROQ_API_KEY"`.
    /// This field is a *name*, never a value — NFR-SECURITY-001.
    pub env_key_var: String,
    /// Whether the provider is OpenAI-compatible or needs a bespoke impl.
    pub compat_kind: CompatKind,
    /// Default model id, e.g. `"llama-3.3-70b-versatile"`.
    pub default_model: String,
    /// Capability bitmap (streaming, tool_calling, vision, embeddings,
    /// json_mode).
    pub supports: ProviderFeatures,
    /// Extra static headers to attach to every request, e.g.
    /// `{"HTTP-Referer": "https://archon.dev"}` for OpenRouter.
    pub headers: HashMap<String, String>,
    /// TASK-AGS-705: per-provider wire quirks (Groq tool-call format,
    /// DeepSeek logprobs-strip, Mistral NDJSON delimiter). Skipped from
    /// serde round-trips because quirks are an internal implementation
    /// detail — users never configure them via TOML/YAML. The `default`
    /// attribute fills in `ProviderQuirks::DEFAULT` on deserialize.
    #[serde(skip, default)]
    pub quirks: ProviderQuirks,
    /// GHOST-003: true when this provider has no real implementation
    /// (stub/gap provider). `/providers` uses this to mark entries as
    /// not-yet-configurable.
    #[serde(default)]
    pub is_gap: bool,
}
