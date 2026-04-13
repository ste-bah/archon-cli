//! TASK-AGS-710 Gate 1: `dispatch_native` wiring tests.
//!
//! Verifies the native-dispatch arms that TASK-AGS-710 touches for the
//! subset of native ids that are actually resolvable via
//! `LlmConfig::resolve_descriptor()` — i.e. the ids present in
//! `NATIVE_REGISTRY`. Each test uses a UNIQUE env var name via
//! `api_key_env` override so parallel `--test-threads=2` is race-free.
//!
//! TASK-AGS-710 SPEC DEVIATION (Gate 1 tests subset):
//!   The original test plan included `test_dispatch_local_wired` and
//!   `test_dispatch_vertex_arch_error`. Neither `local` nor `vertex` is
//!   present in `NATIVE_REGISTRY` (verified: the 9 native entries are
//!   openai/anthropic/gemini/xai/bedrock/azure/cohere/copilot/minimax) nor
//!   in `OPENAI_COMPAT_REGISTRY`, so `resolve_descriptor()` fails with
//!   `MissingCredential { var: "unknown provider: local|vertex" }` BEFORE
//!   `dispatch_native` is ever reached. Those two tests would therefore
//!   exercise `resolve_descriptor`, not `dispatch_native`, and give false
//!   positives about native wiring. They are dropped here; `dispatch_native`
//!   still contains defensive arms for `local`/`vertex` to protect against
//!   future registry additions (see `builder.rs` inline comments).
//!
//!   The `local` legacy path is covered by `runtime/llm.rs::tests::
//!   test_local_provider_constructs_without_fallback` at the adapter
//!   wrapper layer, which is the path archon-cli actually takes for
//!   `cfg.provider = "local"`.

use std::sync::Arc;

use archon_llm::config::LlmConfig;
use archon_llm::providers::{build_llm_provider, ProviderError};

fn http() -> Arc<reqwest::Client> {
    Arc::new(reqwest::Client::new())
}

/// Temporarily set an env var for the duration of a test closure. Each test
/// uses a UNIQUE var name so tests can run under `--test-threads=2` without
/// stepping on each other.
fn with_env<F: FnOnce()>(var: &str, val: &str, f: F) {
    // SAFETY: std::env::set_var is marked unsafe in Rust 2024 edition; each
    // test here scopes mutation to a unique var name, so interleaved tests
    // do not observe each other's env state.
    unsafe {
        std::env::set_var(var, val);
    }
    f();
    unsafe {
        std::env::remove_var(var);
    }
}

// ---------------------------------------------------------------------------
// xai — wired via OpenAiCompatProvider (base_url = https://api.x.ai/v1).
// xai is in NATIVE_REGISTRY with compat_kind = Native, but the wire
// protocol is OpenAI-compatible so `dispatch_native` routes it through
// `OpenAiCompatProvider::new(descriptor, http, api_key)`.
// ---------------------------------------------------------------------------

#[test]
fn test_dispatch_xai_wired_as_compat() {
    const XAI_VAR: &str = "XAI_API_KEY_DNW_T2";
    with_env(XAI_VAR, "test-xai-key", || {
        let cfg = LlmConfig {
            provider: "xai".into(),
            model: None,
            base_url: None,
            api_key_env: Some(XAI_VAR.into()),
            retry: None,
        };
        // NOTE: Result::expect requires E: Debug on the Ok type, but
        // `Arc<dyn LlmProvider>` is not Debug. Drain via match.
        let p = match build_llm_provider(&cfg, http()) {
            Ok(p) => p,
            Err(e) => panic!("xai must build, got error: {e}"),
        };
        assert!(
            !p.name().is_empty(),
            "xai compat provider must have a non-empty name"
        );
    });
}

// ---------------------------------------------------------------------------
// anthropic / bedrock / gemini — explicit architectural errors
//
// These three natives are in NATIVE_REGISTRY so they DO reach
// `dispatch_native`, but their constructors require config shapes that the
// flat `archon_llm::LlmConfig` cannot express (AnthropicClient, SigV4
// region+model_id, x-goog-api-key + GAI endpoint). `dispatch_native` must
// surface a clear architectural error instead of the legacy stub.
// ---------------------------------------------------------------------------

fn assert_arch_error(cfg: &LlmConfig, must_not_contain: &str) {
    // NOTE: `Result::expect_err` requires `T: Debug`, but
    // `Arc<dyn LlmProvider>` is not Debug. Drain via match instead.
    let err = match build_llm_provider(cfg, http()) {
        Ok(_) => panic!("expected ProviderError, got Ok(provider)"),
        Err(e) => e,
    };
    let msg = format!("{err}");
    assert!(
        matches!(err, ProviderError::InvalidResponse { .. }),
        "expected ProviderError::InvalidResponse, got {err:?}"
    );
    assert!(
        !msg.contains(must_not_contain),
        "error must not contain {must_not_contain:?}; got: {msg}"
    );
}

#[test]
fn test_dispatch_anthropic_arch_error() {
    const VAR: &str = "ANTHROPIC_API_KEY_DNW_T3";
    with_env(VAR, "test", || {
        let cfg = LlmConfig {
            provider: "anthropic".into(),
            model: None,
            base_url: None,
            api_key_env: Some(VAR.into()),
            retry: None,
        };
        assert_arch_error(&cfg, "deferred");
    });
}

#[test]
fn test_dispatch_bedrock_arch_error() {
    const VAR: &str = "AWS_ACCESS_KEY_ID_DNW_T4";
    with_env(VAR, "test", || {
        let cfg = LlmConfig {
            provider: "bedrock".into(),
            model: None,
            base_url: None,
            api_key_env: Some(VAR.into()),
            retry: None,
        };
        assert_arch_error(&cfg, "deferred");
    });
}

#[test]
fn test_dispatch_gemini_arch_error() {
    const VAR: &str = "GEMINI_API_KEY_DNW_T6";
    with_env(VAR, "test", || {
        let cfg = LlmConfig {
            provider: "gemini".into(),
            model: None,
            base_url: None,
            api_key_env: Some(VAR.into()),
            retry: None,
        };
        assert_arch_error(&cfg, "deferred");
    });
}
