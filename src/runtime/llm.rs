//! LLM provider construction.
//!
//! TASK-AGS-699: extracted verbatim from `main.rs` (formerly around line 4552)
//! as a prerequisite for Phase 7 multi-provider work. The body is preserved
//! byte-for-byte from the original; Phase 7 TASK-AGS-710 replaces it in place
//! with the registry-driven `build_llm_provider` from TECH-AGS-PROVIDERS.

use std::sync::Arc;

use archon_core::config::LlmConfig;
use archon_llm::anthropic::AnthropicClient;
use archon_llm::provider::LlmProvider;

/// Build the active LLM provider from the `[llm]` config section.
///
/// Matches on `llm_cfg.provider` to construct the appropriate provider.
/// Falls back to Anthropic when the selected provider is missing required
/// credentials or is unrecognised.
pub(crate) fn build_llm_provider(
    llm_cfg: &LlmConfig,
    api_client: AnthropicClient,
) -> Arc<dyn LlmProvider> {
    use archon_llm::providers::{
        AnthropicProvider, BedrockProvider, LocalProvider, OpenAiProvider, VertexProvider,
    };

    match llm_cfg.provider.as_str() {
        "openai" => {
            let key = llm_cfg.openai.api_key.clone().unwrap_or_default();
            let resolved = OpenAiProvider::resolve_api_key(&key);
            if resolved.is_empty() {
                tracing::warn!("OpenAI selected but no API key found; falling back to Anthropic");
                return Arc::new(AnthropicProvider::new(api_client));
            }
            Arc::new(OpenAiProvider::new(
                key,
                llm_cfg.openai.base_url.clone(),
                llm_cfg.openai.model.clone(),
            ))
        }
        "bedrock" => {
            if llm_cfg.bedrock.region.is_empty() || llm_cfg.bedrock.model_id.is_empty() {
                tracing::warn!(
                    "Bedrock selected but region/model_id missing; falling back to Anthropic"
                );
                return Arc::new(AnthropicProvider::new(api_client));
            }
            Arc::new(BedrockProvider::new(
                llm_cfg.bedrock.region.clone(),
                llm_cfg.bedrock.model_id.clone(),
            ))
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
        _ => {
            // Default / "anthropic" / unknown → Anthropic
            if llm_cfg.provider != "anthropic" {
                tracing::warn!(
                    "Unknown LLM provider '{}'; falling back to Anthropic",
                    llm_cfg.provider
                );
            }
            Arc::new(AnthropicProvider::new(api_client))
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
}
