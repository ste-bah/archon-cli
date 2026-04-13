//! TASK-AGS-706 Gate 1: Tests-first for `LlmConfig` + `build_llm_provider`.
//!
//! Written BEFORE implementation so the impl has a compile-and-pass target.
//!
//! Spec deviation (greenlit 2026-04-13, TASK-AGS-703 header):
//!   ProviderError -> LlmError at the trait boundary, but `build_llm_provider`
//!   is NOT on the trait. Spec line 1116 returns `Arc<dyn LlmProvider>`; we
//!   return `Result<Arc<dyn LlmProvider>, ProviderError>` so credential-miss
//!   paths surface as `ProviderError::MissingCredential` as the spec requires
//!   (see Validation Criteria 5 + 7).

use std::sync::Arc;

use archon_llm::config::LlmConfig;
use archon_llm::providers::{
    build_llm_provider, CompatKind, ProviderError,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn http() -> Arc<reqwest::Client> {
    Arc::new(reqwest::Client::new())
}

/// Temporarily set an env var for the duration of a test.
///
/// Tests that manipulate env vars cannot run concurrently on the same var,
/// but cargo `--test-threads=2` is fine because each test uses a unique var.
fn with_env<F: FnOnce()>(var: &str, val: &str, f: F) {
    // SAFETY: std::env::set_var is marked unsafe in Rust 2024; tests run
    // serialized per-var because we scope the var to this call only.
    unsafe {
        std::env::set_var(var, val);
    }
    f();
    unsafe {
        std::env::remove_var(var);
    }
}

// ---------------------------------------------------------------------------
// LlmConfig deserialization + resolve_descriptor
// ---------------------------------------------------------------------------

#[test]
fn toml_groq_deserializes_and_resolves_to_compat_descriptor() {
    let toml_src = r#"
provider = "groq"
model = "llama-3.3-70b-versatile"
"#;
    let cfg: LlmConfig = toml::from_str(toml_src).expect("toml parses");
    assert_eq!(cfg.provider, "groq");
    assert_eq!(cfg.model.as_deref(), Some("llama-3.3-70b-versatile"));

    let desc = cfg
        .resolve_descriptor()
        .expect("groq must resolve via shorthand auto-routing");
    assert_eq!(desc.id, "groq");
    assert!(matches!(desc.compat_kind, CompatKind::OpenAiCompat));
}

#[test]
fn provider_openai_backward_compat_resolves_to_native() {
    let cfg = LlmConfig {
        provider: "openai".into(),
        model: None,
        base_url: None,
        api_key_env: None,
    };
    let desc = cfg
        .resolve_descriptor()
        .expect("openai must continue to route post-migration (NFR-ARCH-002)");
    assert_eq!(desc.id, "openai");
    assert!(matches!(desc.compat_kind, CompatKind::Native));
}

#[test]
fn openai_compat_prefix_resolves_to_compat_descriptor() {
    let cfg = LlmConfig {
        provider: "openai-compat:deepseek".into(),
        model: None,
        base_url: None,
        api_key_env: None,
    };
    let desc = cfg
        .resolve_descriptor()
        .expect("openai-compat:deepseek must resolve");
    assert_eq!(desc.id, "deepseek");
    assert!(matches!(desc.compat_kind, CompatKind::OpenAiCompat));
}

#[test]
fn unknown_provider_returns_missing_credential_with_message() {
    let cfg = LlmConfig {
        provider: "nonexistent".into(),
        model: None,
        base_url: None,
        api_key_env: None,
    };
    let err = cfg
        .resolve_descriptor()
        .expect_err("unknown id must Err");
    match err {
        ProviderError::MissingCredential { var } => {
            assert!(
                var.contains("unknown provider: nonexistent"),
                "error must carry 'unknown provider: nonexistent' marker; got: {var}"
            );
        }
        other => panic!("expected MissingCredential, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// build_llm_provider — compat happy path and credential errors
// ---------------------------------------------------------------------------

#[test]
fn build_llm_provider_with_groq_api_key_returns_compat_provider() {
    with_env("GROQ_API_KEY", "test-groq-key", || {
        let cfg = LlmConfig {
            provider: "groq".into(),
            model: Some("llama-3.3-70b-versatile".into()),
            base_url: None,
            api_key_env: None,
        };
        let provider = build_llm_provider(&cfg, http())
            .expect("groq must build when GROQ_API_KEY is set");
        assert_eq!(
            provider.name(),
            "Groq",
            "display_name from registry must surface through LlmProvider::name()"
        );
    });
}

#[test]
fn build_llm_provider_without_groq_api_key_errors_with_var_name() {
    // Make sure the var is NOT set.
    unsafe {
        std::env::remove_var("GROQ_API_KEY_ZZ_ABSENT");
    }
    let cfg = LlmConfig {
        provider: "groq".into(),
        model: None,
        base_url: None,
        // Force a deterministic var name that we can guarantee is unset.
        api_key_env: Some("GROQ_API_KEY_ZZ_ABSENT".into()),
    };
    // Avoid `expect_err` because `Arc<dyn LlmProvider>` is not `Debug`.
    match build_llm_provider(&cfg, http()) {
        Ok(_) => panic!("missing env var must Err, not Ok"),
        Err(ProviderError::MissingCredential { var }) => {
            assert_eq!(var, "GROQ_API_KEY_ZZ_ABSENT");
        }
        Err(other) => panic!("expected MissingCredential, got {other:?}"),
    }
}

#[test]
fn build_llm_provider_auth_flavor_none_requires_no_env_var() {
    // ollama is AuthFlavor::None in the compat registry — builder must
    // succeed without touching any env var.
    let cfg = LlmConfig {
        provider: "ollama".into(),
        model: None,
        base_url: None,
        api_key_env: None,
    };
    let provider = build_llm_provider(&cfg, http())
        .expect("ollama AuthFlavor::None must build without env var");
    assert!(
        !provider.name().is_empty(),
        "ollama must have non-empty display_name"
    );
}

// ---------------------------------------------------------------------------
// Invariant: exactly one `match.*descriptor.id` site in builder.rs
// ---------------------------------------------------------------------------

#[test]
fn builder_has_exactly_one_match_on_descriptor_id() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let path = format!("{manifest_dir}/src/providers/builder.rs");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("must read {path}: {e}"));

    // Count occurrences of `match` followed by anything and `descriptor.id`.
    let mut count = 0usize;
    for line in src.lines() {
        let trimmed = line.trim_start();
        // Skip comment lines so doc comments describing the invariant
        // don't inflate the count.
        if trimmed.starts_with("//") {
            continue;
        }
        if trimmed.contains("match") && trimmed.contains("descriptor.id") {
            count += 1;
        }
    }
    assert_eq!(
        count, 1,
        "builder.rs must contain exactly ONE `match ... descriptor.id` site; found {count}"
    );
}

// ---------------------------------------------------------------------------
// Serde round-trip for LlmConfig (NFR-ARCH-002 TOML + YAML support)
// ---------------------------------------------------------------------------

#[test]
fn llm_config_round_trips_through_toml() {
    let cfg = LlmConfig {
        provider: "openai".into(),
        model: Some("gpt-4o".into()),
        base_url: None,
        api_key_env: Some("OPENAI_API_KEY".into()),
    };
    let s = toml::to_string(&cfg).expect("serialize");
    let back: LlmConfig = toml::from_str(&s).expect("deserialize");
    assert_eq!(back.provider, cfg.provider);
    assert_eq!(back.model, cfg.model);
    assert_eq!(back.api_key_env, cfg.api_key_env);
}
