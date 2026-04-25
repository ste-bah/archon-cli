//! `ProviderFeatures` — pure data describing which capabilities a provider
//! advertises.
//!
//! TECH-AGS-PROVIDERS lines 1076-1082. This is a plural sibling of the
//! existing `provider::ProviderFeature` enum (singular) used by the current
//! `LlmProvider::supports_feature` API. They coexist: the enum is a runtime
//! capability query, this struct is a static registry descriptor field.

/// Capability bitmap for a provider descriptor.
///
/// All fields are intentionally `bool` rather than `Option<bool>` — an absent
/// capability is `false`, never unknown. This keeps descriptor tables terse
/// and deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderFeatures {
    /// Server-sent event / chunked streaming.
    pub streaming: bool,
    /// Function / tool calling (OpenAI-style `tools` field).
    pub tool_calling: bool,
    /// Multi-modal image input.
    pub vision: bool,
    /// Embeddings endpoint.
    pub embeddings: bool,
    /// Structured JSON output / `response_format = json_object`.
    pub json_mode: bool,
}

impl ProviderFeatures {
    /// All capabilities off. Use when a descriptor genuinely advertises nothing
    /// (e.g. a placeholder entry pending confirmation).
    pub const fn none() -> Self {
        Self {
            streaming: false,
            tool_calling: false,
            vision: false,
            embeddings: false,
            json_mode: false,
        }
    }

    /// Minimal streaming chat: `streaming = true`, everything else off. The
    /// common baseline for OpenAI-compatible providers whose tool-calling /
    /// vision / embedding support is either absent or not yet verified.
    pub const fn chat_only() -> Self {
        Self {
            streaming: true,
            tool_calling: false,
            vision: false,
            embeddings: false,
            json_mode: false,
        }
    }
}
