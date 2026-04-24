//! Legacy LLM provider dispatch bridging `archon_core::config::LlmConfig`
//! (nested, on-disk TOML shape) → `archon_llm` providers. Preserved as a
//! thin adapter wrapper; the body was originally extracted from `main.rs`
//! by TASK-AGS-699, and TASK-AGS-710 now routes new flat-config providers
//! through `archon_llm::ActiveProvider` while keeping the 5 legacy natives
//! on their hand-rolled constructors.
//!
//! TASK-AGS-710 SPEC DEVIATION (greenlit 2026-04-13, same precedent as
//! TASK-AGS-700/703):
//!
//! TECH-AGS-PROVIDERS §1138 + the TASK-AGS-710 spec specify replacing the
//! dispatch block with `ActiveProvider::new(&cfg.llm, http.clone())`
//! called from `main.rs`, assuming `cfg.llm` is the new flat
//! `archon_llm::LlmConfig`. Four real-world mismatches force this adapter
//! layer to stay:
//!
//! 1. `cfg.llm` is `archon_core::config::LlmConfig` — a NESTED shape with
//!    per-provider subconfigs (`openai{}`, `bedrock{}`, `vertex{}`,
//!    `local{}`) matching the on-disk `config.toml` users already have.
//!    Replacing it would break every existing user config. The flat
//!    `archon_llm::LlmConfig` cannot carry `bedrock.region` /
//!    `bedrock.model_id`, `vertex.project_id` / `vertex.credentials_file`,
//!    or `local.timeout_secs` — the per-provider fields simply don't
//!    exist in the flat shape. A `From` impl would have to DROP data.
//!    Rejected.
//!
//! 2. The new `archon_llm` builder hard-errors on missing credentials
//!    (`ProviderError::MissingCredential`). The legacy `build_llm_provider`
//!    falls back to Anthropic with a `tracing::warn!`. Users running with
//!    incomplete `.openai.api_key` silently work today; hard-error is a
//!    silent behavior regression (P0). The wrapper below absorbs the
//!    fallback semantics so `ActiveProvider` can stay strict. This inverts
//!    TASK-AGS-710 Validation Criterion 7 (hard-error on missing
//!    `GROQ_API_KEY` → now: warn + Anthropic fallback, preserving legacy
//!    infallible `-> Arc<dyn LlmProvider>` signature).
//!
//! 3. `archon-llm` does NOT depend on `archon-core`, so the adapter lives
//!    here as a free function rather than as `impl From<&archon_core::...>
//!    for archon_llm::...` on the `archon_llm` side. Strictly equivalent,
//!    avoids expanding the `archon_llm` crate graph.
//!
//! 4. "Expose active handle to agent-loop state so `/model` can swap"
//!    (spec scope bullet 4) is postponed to phase-8 alongside `/model`
//!    itself — the return type stays `Arc<dyn LlmProvider>` so the 3
//!    `main.rs` call sites at L473/L2300/L3179 do not change. Changing the
//!    return type to a tuple would touch all three call sites and is out
//!    of scope for this fix.
//!
//! Net result: `main.rs` call sites unchanged, user config-file format
//! unchanged, `archon_llm` builder/`ActiveProvider` stay pure, Anthropic
//! fallback preserved on every legacy code path.

use std::sync::Arc;

use archon_core::config::LlmConfig;
use archon_llm::anthropic::AnthropicClient;
use archon_llm::provider::LlmProvider;
use archon_llm::providers::{
    AnthropicProvider, BedrockProvider, LocalProvider, OpenAiProvider, ProviderError,
    VertexProvider,
};
use archon_llm::{ActiveProvider, LlmConfig as FlatLlmConfig};

/// Build the active LLM provider from the `[llm]` config section.
///
/// Matches on `llm_cfg.provider` to construct the appropriate provider.
/// - The 5 legacy natives (`anthropic`, `openai`, `bedrock`, `vertex`,
///   `local`) stay on their hand-rolled constructors so the nested
///   `archon_core::config::LlmConfig` sub-fields are honored.
/// - Any other provider string (`groq`, `deepseek`, `mistral`, `xai`,
///   `gemini`, `azure`, `cohere`, `copilot`, `minimax`, `together`,
///   `perplexity`, ...) is routed through `archon_llm::ActiveProvider`
///   with a flat `archon_llm::LlmConfig`.
///
/// Falls back to Anthropic (with a `tracing::warn!`) whenever the selected
/// provider is missing required credentials, is unrecognised, or fails to
/// construct for any other reason. The return type is intentionally
/// infallible so the three `main.rs` call sites remain untouched.
pub(crate) fn build_llm_provider(
    llm_cfg: &LlmConfig,
    api_client: AnthropicClient,
) -> Arc<dyn LlmProvider> {
    match llm_cfg.provider.as_str() {
        "anthropic" => Arc::new(AnthropicProvider::new(api_client)),

        "openai" => {
            let inline_key = llm_cfg.openai.api_key.clone().unwrap_or_default();
            let resolved = OpenAiProvider::resolve_api_key(&inline_key);
            if resolved.is_empty() {
                tracing::warn!("OpenAI selected but no API key found; falling back to Anthropic");
                return Arc::new(AnthropicProvider::new(api_client));
            }
            Arc::new(OpenAiProvider::new(
                resolved,
                llm_cfg.openai.base_url.clone(),
                llm_cfg.openai.model.clone(),
            ))
        }

        "bedrock" => {
            let region = llm_cfg.bedrock.region.clone();
            let model_id = llm_cfg.bedrock.model_id.clone();
            if region.is_empty() || model_id.is_empty() {
                tracing::warn!(
                    "Bedrock selected but region/model_id missing; falling back to Anthropic"
                );
                return Arc::new(AnthropicProvider::new(api_client));
            }
            Arc::new(BedrockProvider::new(region, model_id))
        }

        "vertex" => {
            let project_id = llm_cfg.vertex.project_id.as_deref().unwrap_or("");
            if project_id.is_empty() {
                tracing::warn!("Vertex selected but project_id missing; falling back to Anthropic");
                return Arc::new(AnthropicProvider::new(api_client));
            }
            let publisher = if llm_cfg.vertex.model.contains("claude") {
                "anthropic"
            } else {
                "google"
            };
            Arc::new(VertexProvider::new(
                project_id.to_string(),
                llm_cfg.vertex.region.clone(),
                llm_cfg.vertex.model.clone(),
                publisher.to_string(),
                llm_cfg.vertex.credentials_file.clone(),
            ))
        }

        "local" => Arc::new(LocalProvider::new(
            llm_cfg.local.base_url.clone(),
            llm_cfg.local.model.clone(),
            llm_cfg.local.timeout_secs,
            llm_cfg.local.pull_if_missing,
        )),

        other => {
            // Flat-config descriptor path: groq, deepseek, mistral, xai,
            // gemini, azure, cohere, copilot, minimax, together,
            // perplexity, etc. Route through `archon_llm::ActiveProvider`
            // with a minimal flat LlmConfig; credentials come from the
            // descriptor's default env var (api_key_env override not
            // supported via nested archon_core::config::LlmConfig yet).
            let flat = FlatLlmConfig {
                provider: other.to_string(),
                model: None,
                base_url: None,
                api_key_env: None,
                retry: None,
            };
            let http = Arc::new(reqwest::Client::new());
            match ActiveProvider::new(&flat, http) {
                Ok(active) => Arc::new(active),
                Err(ProviderError::MissingCredential { var }) => {
                    tracing::warn!(
                        provider = %other,
                        env_var = %var,
                        "provider credentials missing; falling back to Anthropic"
                    );
                    Arc::new(AnthropicProvider::new(api_client))
                }
                Err(e) => {
                    tracing::warn!(
                        provider = %other,
                        error = %e,
                        "provider construction failed; falling back to Anthropic"
                    );
                    Arc::new(AnthropicProvider::new(api_client))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_llm::auth::AuthProvider;
    use archon_llm::identity::{IdentityMode, IdentityProvider};
    use archon_llm::types::Secret;
    use std::sync::Mutex;

    /// Serialises env-mutating tests. Module-local because no other test in
    /// the `archon-cli-workspace` binary-crate test target touches
    /// `OPENAI_API_KEY`.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn make_test_client() -> AnthropicClient {
        let auth = AuthProvider::ApiKey(Secret::new("test-key".into()));
        let identity = IdentityProvider::new(
            IdentityMode::Clean,
            "session".into(),
            "device".into(),
            String::new(),
        );
        AnthropicClient::new(auth, identity, None)
    }

    #[test]
    fn unknown_provider_falls_back_to_anthropic() {
        let mut cfg = LlmConfig::default();
        cfg.provider = "__ags699_unknown__".into();
        let provider = build_llm_provider(&cfg, make_test_client());
        assert_eq!(provider.name(), "anthropic");
    }

    #[test]
    fn openai_with_empty_key_falls_back_to_anthropic() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("OPENAI_API_KEY").ok();
        // SAFETY: `ENV_LOCK` serialises env mutations inside this module and
        // no other test in this crate's test binary touches `OPENAI_API_KEY`.
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }

        let mut cfg = LlmConfig::default();
        cfg.provider = "openai".into();
        cfg.openai.api_key = None;
        let provider = build_llm_provider(&cfg, make_test_client());
        assert_eq!(provider.name(), "anthropic");

        if let Some(v) = prev {
            // SAFETY: see above; restoring prior env state.
            unsafe {
                std::env::set_var("OPENAI_API_KEY", v);
            }
        }
    }

    #[test]
    fn bedrock_with_missing_region_falls_back_to_anthropic() {
        let mut cfg = LlmConfig::default();
        cfg.provider = "bedrock".into();
        cfg.bedrock.region = String::new();
        let provider = build_llm_provider(&cfg, make_test_client());
        assert_eq!(provider.name(), "anthropic");
    }

    // ---- TASK-AGS-710 Gate 1 (tests-first) --------------------------------

    #[test]
    fn test_anthropic_provider_explicit_returns_anthropic() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut cfg = LlmConfig::default();
        cfg.provider = "anthropic".to_string();
        let provider = build_llm_provider(&cfg, make_test_client());
        assert_eq!(provider.name(), "anthropic");
    }

    #[test]
    fn test_local_provider_constructs_without_fallback() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut cfg = LlmConfig::default();
        cfg.provider = "local".to_string();
        let provider = build_llm_provider(&cfg, make_test_client());
        // LocalProvider::name() returns "local" (verified in
        // crates/archon-llm/src/providers/local.rs ~line 227).
        assert_eq!(provider.name(), "local");
    }

    #[test]
    fn test_groq_without_env_falls_back_to_anthropic() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("GROQ_API_KEY").ok();
        // SAFETY: single-threaded via ENV_LOCK above.
        unsafe {
            std::env::remove_var("GROQ_API_KEY");
        }
        let mut cfg = LlmConfig::default();
        cfg.provider = "groq".to_string();
        let provider = build_llm_provider(&cfg, make_test_client());
        assert_eq!(
            provider.name(),
            "anthropic",
            "missing groq env var must fall back to Anthropic (legacy behavior preserved; \
             spec Criterion 7 hard-error is deviated)"
        );
        if let Some(v) = prev {
            // SAFETY: restore previous env state, still guarded by ENV_LOCK.
            unsafe {
                std::env::set_var("GROQ_API_KEY", v);
            }
        }
    }

    #[test]
    fn test_unknown_flat_provider_falls_back_to_anthropic() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut cfg = LlmConfig::default();
        cfg.provider = "definitely-not-a-real-provider-zzz".to_string();
        let provider = build_llm_provider(&cfg, make_test_client());
        assert_eq!(provider.name(), "anthropic");
    }
}
