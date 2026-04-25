//! TASK-AGS-709 Gate 1: tests-first for `ActiveProvider` — an
//! ArcSwap-backed live-swappable handle that delegates `LlmProvider`
//! calls to whichever provider is currently active.
//!
//! Covers Validation Criteria 2, 3, 5. Concurrent-swap coverage
//! (Criterion 4) lives in `tests/active_concurrent.rs`.
//!
//! Each test uses a UNIQUE env var name via `api_key_env` override so
//! parallel cargo test (--test-threads=2) is race-free. Never touch
//! the real `GROQ_API_KEY` / `DEEPSEEK_API_KEY` — those might also be
//! manipulated by neighboring test binaries.

use std::sync::Arc;

use archon_llm::active::ActiveProvider;
use archon_llm::config::LlmConfig;
use archon_llm::provider::LlmProvider;
use archon_llm::providers::ProviderError;

fn http() -> Arc<reqwest::Client> {
    Arc::new(reqwest::Client::new())
}

/// Set a unique env var for the duration of a closure. Safe under
/// parallel test execution as long as each test uses a distinct var.
fn with_env<F: FnOnce()>(var: &str, val: &str, f: F) {
    unsafe {
        std::env::set_var(var, val);
    }
    f();
    unsafe {
        std::env::remove_var(var);
    }
}

// ---------------------------------------------------------------------------
// Criterion 2: swap changes the active descriptor
// ---------------------------------------------------------------------------

#[test]
fn test_swap_changes_descriptor() {
    const GROQ_VAR: &str = "GROQ_API_KEY_AP_T1";
    const DS_VAR: &str = "DEEPSEEK_API_KEY_AP_T1";
    with_env(GROQ_VAR, "test-groq", || {
        with_env(DS_VAR, "test-deepseek", || {
            let cfg_groq = LlmConfig {
                provider: "groq".into(),
                model: None,
                base_url: None,
                api_key_env: Some(GROQ_VAR.into()),
                retry: None,
            };
            let active = ActiveProvider::new(&cfg_groq, http()).expect("groq cfg builds");
            assert_eq!(active.current_descriptor().id, "groq");

            let cfg_deepseek = LlmConfig {
                provider: "deepseek".into(),
                model: None,
                base_url: None,
                api_key_env: Some(DS_VAR.into()),
                retry: None,
            };
            let new_desc = active
                .swap(&cfg_deepseek)
                .expect("swap to deepseek must succeed");
            assert_eq!(new_desc.id, "deepseek");
            assert_eq!(
                active.current_descriptor().id,
                "deepseek",
                "current_descriptor must reflect the post-swap provider"
            );
        });
    });
}

// ---------------------------------------------------------------------------
// Criterion 3: swap failure leaves the old provider unchanged (EC-PROV-02)
// ---------------------------------------------------------------------------

#[test]
fn test_swap_failure_preserves_old_provider() {
    const GROQ_VAR: &str = "GROQ_API_KEY_AP_T2";
    with_env(GROQ_VAR, "test-groq", || {
        let cfg_groq = LlmConfig {
            provider: "groq".into(),
            model: None,
            base_url: None,
            api_key_env: Some(GROQ_VAR.into()),
            retry: None,
        };
        let active = ActiveProvider::new(&cfg_groq, http()).expect("groq cfg builds");
        assert_eq!(active.current_descriptor().id, "groq");

        let cfg_bad = LlmConfig {
            provider: "nonexistent-provider-xyz".into(),
            model: None,
            base_url: None,
            api_key_env: None,
            retry: None,
        };
        let err = active
            .swap(&cfg_bad)
            .expect_err("unknown provider must Err");
        match err {
            ProviderError::MissingCredential { var } => {
                assert!(
                    var.contains("unknown provider"),
                    "error must carry 'unknown provider' marker; got: {var}"
                );
            }
            other => panic!("expected MissingCredential, got {other:?}"),
        }

        assert_eq!(
            active.current_descriptor().id,
            "groq",
            "failed swap must leave the old descriptor in place"
        );
    });
}

// ---------------------------------------------------------------------------
// Criterion 5: trait-object check — ActiveProvider IS an LlmProvider
// ---------------------------------------------------------------------------

#[test]
fn test_active_provider_impls_llm_provider() {
    const GROQ_VAR: &str = "GROQ_API_KEY_AP_T3";
    with_env(GROQ_VAR, "test-groq", || {
        let cfg = LlmConfig {
            provider: "groq".into(),
            model: None,
            base_url: None,
            api_key_env: Some(GROQ_VAR.into()),
            retry: None,
        };
        let active = ActiveProvider::new(&cfg, http()).expect("groq builds");
        let as_trait_object: Arc<dyn LlmProvider> = Arc::new(active);
        assert!(
            !as_trait_object.name().is_empty(),
            "delegated name() must surface the current provider"
        );
    });
}

// ---------------------------------------------------------------------------
// Build-time failure must also preserve old provider.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_missing_credential_preserves_old_provider() {
    const GROQ_VAR: &str = "GROQ_API_KEY_AP_T4";
    const ABSENT_VAR: &str = "DEEPSEEK_ZZ_ABSENT_AP_T4";
    with_env(GROQ_VAR, "test-groq", || {
        unsafe {
            std::env::remove_var(ABSENT_VAR);
        }
        let cfg_groq = LlmConfig {
            provider: "groq".into(),
            model: None,
            base_url: None,
            api_key_env: Some(GROQ_VAR.into()),
            retry: None,
        };
        let active = ActiveProvider::new(&cfg_groq, http()).expect("groq builds");

        let cfg_deepseek_no_env = LlmConfig {
            provider: "deepseek".into(),
            model: None,
            base_url: None,
            api_key_env: Some(ABSENT_VAR.into()),
            retry: None,
        };
        let err = active
            .swap(&cfg_deepseek_no_env)
            .expect_err("missing env var must Err");
        match err {
            ProviderError::MissingCredential { var } => {
                assert_eq!(var, ABSENT_VAR);
            }
            other => panic!("expected MissingCredential, got {other:?}"),
        }

        assert_eq!(
            active.current_descriptor().id,
            "groq",
            "build-time failure must also preserve old provider"
        );
    });
}

// ---------------------------------------------------------------------------
// current() returns Arc clones of the same backing provider
// ---------------------------------------------------------------------------

#[test]
fn test_current_returns_cloned_arc() {
    const GROQ_VAR: &str = "GROQ_API_KEY_AP_T5";
    with_env(GROQ_VAR, "test-groq", || {
        let cfg = LlmConfig {
            provider: "groq".into(),
            model: None,
            base_url: None,
            api_key_env: Some(GROQ_VAR.into()),
            retry: None,
        };
        let active = ActiveProvider::new(&cfg, http()).expect("groq builds");
        let p1 = active.current();
        let p2 = active.current();
        assert!(
            Arc::ptr_eq(&p1, &p2),
            "current() must return Arc clones of the same backing provider"
        );
    });
}
