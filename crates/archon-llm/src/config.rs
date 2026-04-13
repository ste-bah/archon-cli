//! TASK-AGS-706: `LlmConfig` schema + `resolve_descriptor()` routing logic.
//!
//! Spec: 02-technical-spec.md TECH-AGS-PROVIDERS api_contracts (lines
//! 1116-1126, 1159-1161). The config is serde-compatible with both TOML
//! and YAML (spec line 1121 "Config (TOML / YAML)") and carries only the
//! fields needed to pick a descriptor and locate its credential env var.
//!
//! Backward-compat (NFR-ARCH-002, spec lines 1159-1161):
//! - Existing `provider = "openai"` continues to resolve to the native
//!   openai descriptor in `NATIVE_REGISTRY`.
//! - New compat providers reachable via `provider = "openai-compat:<id>"`.
//! - Shorthand `provider = "<id>"` auto-routes to `OPENAI_COMPAT_REGISTRY`
//!   when the id isn't in `NATIVE_REGISTRY`.

use serde::{Deserialize, Serialize};
use url::Url;

use crate::providers::{
    get_compat, get_native, ProviderDescriptor, ProviderError,
};

/// Runtime configuration for the chosen LLM provider.
///
/// Serde-compatible with TOML and YAML. Only `provider` is required; the
/// remaining fields override descriptor defaults when present.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Either a bare provider id (`"openai"`, `"groq"`) or the explicit
    /// `"openai-compat:<id>"` prefix form.
    pub provider: String,

    /// Optional model override. Falls back to `descriptor.default_model`
    /// when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Optional base URL override. Reserved for TASK-AGS-710 when the
    /// dispatcher learns to respect it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<Url>,

    /// Optional env var name override. Falls back to
    /// `descriptor.env_key_var` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
}

impl LlmConfig {
    /// Resolve the configured `provider` string to a `&'static
    /// ProviderDescriptor`. Precedence:
    ///
    /// 1. `openai-compat:<id>` prefix → `OPENAI_COMPAT_REGISTRY[id]`
    /// 2. Exact native id → `NATIVE_REGISTRY[id]` (native wins over
    ///    any same-id compat entry, which implicitly handles the xai
    ///    dual-registration flagged during TASK-AGS-705)
    /// 3. Fallback shorthand → `OPENAI_COMPAT_REGISTRY[id]`
    /// 4. Not found → `ProviderError::MissingCredential` with an
    ///    `"unknown provider: <id>"` marker (spec Validation Criteria 5)
    pub fn resolve_descriptor(&self) -> Result<&'static ProviderDescriptor, ProviderError> {
        const COMPAT_PREFIX: &str = "openai-compat:";

        if let Some(rest) = self.provider.strip_prefix(COMPAT_PREFIX) {
            if let Some(d) = get_compat(rest) {
                return Ok(d);
            }
            return Err(ProviderError::MissingCredential {
                var: format!("unknown provider: {}", self.provider),
            });
        }

        if let Some(d) = get_native(self.provider.as_str()) {
            return Ok(d);
        }

        if let Some(d) = get_compat(self.provider.as_str()) {
            return Ok(d);
        }

        Err(ProviderError::MissingCredential {
            var: format!("unknown provider: {}", self.provider),
        })
    }
}
