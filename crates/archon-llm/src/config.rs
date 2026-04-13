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

use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::providers::{
    get_compat, get_native, ProviderDescriptor, ProviderError,
};
use crate::retry::RetryPolicy;

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

    /// TASK-AGS-708: optional retry policy override. When absent,
    /// `build_llm_provider` uses `RetryPolicy::default()` (3 attempts,
    /// 500ms initial backoff, 8s cap, 2x multiplier, ±25% jitter) which
    /// matches ERR-PROV-02.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicyConfig>,
}

/// Serde-friendly mirror of `RetryPolicy` that uses plain integer seconds
/// and milliseconds instead of `std::time::Duration` (which doesn't have a
/// clean serde representation). Converted via `From`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicyConfig {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub multiplier: f64,
    pub jitter: bool,
}

impl From<RetryPolicyConfig> for RetryPolicy {
    fn from(c: RetryPolicyConfig) -> Self {
        Self {
            max_attempts: c.max_attempts,
            initial_backoff: Duration::from_millis(c.initial_backoff_ms),
            max_backoff: Duration::from_millis(c.max_backoff_ms),
            multiplier: c.multiplier,
            jitter: c.jitter,
        }
    }
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
