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
    let cfg = LlmConfig {
        provider: "__ags699_unknown__".into(),
        ..Default::default()
    };
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

    let mut cfg = LlmConfig {
        provider: "openai".into(),
        ..Default::default()
    };
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
fn openai_fallback_selection_reports_missing_key_reason() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var("OPENAI_API_KEY").ok();
    unsafe {
        std::env::remove_var("OPENAI_API_KEY");
    }

    let mut cfg = LlmConfig {
        provider: "openai".into(),
        ..Default::default()
    };
    cfg.openai.api_key = None;

    let selection = build_llm_provider_selection(&cfg, make_test_client());

    assert_eq!(selection.provider.name(), "anthropic");
    assert_eq!(selection.fallback_reason, Some("openai_missing_api_key"));

    if let Some(v) = prev {
        unsafe {
            std::env::set_var("OPENAI_API_KEY", v);
        }
    }
}

#[test]
fn bedrock_with_missing_region_falls_back_to_anthropic() {
    let mut cfg = LlmConfig {
        provider: "bedrock".into(),
        ..Default::default()
    };
    cfg.bedrock.region = String::new();
    let provider = build_llm_provider(&cfg, make_test_client());
    assert_eq!(provider.name(), "anthropic");
}

#[test]
fn test_anthropic_provider_explicit_returns_anthropic() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = LlmConfig {
        provider: "anthropic".to_string(),
        ..Default::default()
    };
    let provider = build_llm_provider(&cfg, make_test_client());
    assert_eq!(provider.name(), "anthropic");
}

#[test]
fn test_local_provider_constructs_without_fallback() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = LlmConfig {
        provider: "local".to_string(),
        ..Default::default()
    };
    let provider = build_llm_provider(&cfg, make_test_client());
    assert_eq!(provider.name(), "local");
}

#[test]
fn local_provider_constructs_without_anthropic_fallback_client() {
    let cfg = LlmConfig {
        provider: "local".to_string(),
        ..Default::default()
    };

    let provider = build_llm_provider_without_anthropic_fallback(&cfg).unwrap();

    assert_eq!(provider.name(), "local");
}

#[test]
fn openai_missing_key_errors_without_anthropic_fallback_client() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var("OPENAI_API_KEY").ok();
    unsafe {
        std::env::remove_var("OPENAI_API_KEY");
    }
    let mut cfg = LlmConfig {
        provider: "openai".to_string(),
        ..Default::default()
    };
    cfg.openai.api_key = None;

    let error = match build_llm_provider_without_anthropic_fallback(&cfg) {
        Ok(provider) => panic!(
            "expected missing-key error, built provider {}",
            provider.name()
        ),
        Err(error) => error.to_string(),
    };

    assert!(error.contains("OpenAI selected but no API key found"));
    if let Some(v) = prev {
        unsafe {
            std::env::set_var("OPENAI_API_KEY", v);
        }
    }
}

#[test]
fn test_groq_without_env_falls_back_to_anthropic() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var("GROQ_API_KEY").ok();
    // SAFETY: single-threaded via ENV_LOCK above.
    unsafe {
        std::env::remove_var("GROQ_API_KEY");
    }
    let cfg = LlmConfig {
        provider: "groq".to_string(),
        ..Default::default()
    };
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
    let cfg = LlmConfig {
        provider: "definitely-not-a-real-provider-zzz".to_string(),
        ..Default::default()
    };
    let provider = build_llm_provider(&cfg, make_test_client());
    assert_eq!(provider.name(), "anthropic");
}

#[test]
fn unknown_flat_provider_selection_reports_reason() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = LlmConfig {
        provider: "definitely-not-a-real-provider-zzz".to_string(),
        ..Default::default()
    };

    let selection = build_llm_provider_selection(&cfg, make_test_client());

    assert_eq!(selection.provider.name(), "anthropic");
    assert_eq!(
        selection.fallback_reason,
        Some("openai_compatible_unknown_provider")
    );
}
