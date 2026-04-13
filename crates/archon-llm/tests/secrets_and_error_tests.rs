//! TASK-AGS-701 Gate 1: integration tests for the `ApiKey` redactor
//! and `ProviderError` enum. Written before implementation.
//!
//! The in-file unit tests inside `secrets.rs` cover Debug/Display/from_env
//! directly; these integration tests additionally pin the public shape and
//! re-export paths so downstream Phase 7 tasks (702..706) can `use` them
//! without fighting module privacy.

use archon_llm::providers::error::ProviderError;
use archon_llm::ApiKey;

// ---------------------------------------------------------------------------
// ApiKey: redaction invariants
// ---------------------------------------------------------------------------

#[test]
fn api_key_debug_never_exposes_secret() {
    let key = ApiKey::new("sk-super-secret-abc123".to_string());
    let s = format!("{:?}", key);
    assert!(!s.contains("sk-super-secret"));
    assert!(s.contains("***redacted***"), "Debug must contain redaction token: {s}");
    assert!(s.starts_with("ApiKey("), "Debug must prefix with ApiKey(...): {s}");
}

#[test]
fn api_key_display_never_exposes_secret() {
    let key = ApiKey::new("sk-super-secret-abc123".to_string());
    let s = format!("{}", key);
    assert!(!s.contains("sk-super-secret"));
    assert!(s.contains("***redacted***"), "Display must contain redaction token: {s}");
}

#[test]
fn api_key_expose_returns_original() {
    let key = ApiKey::new("sk-raw-value".to_string());
    assert_eq!(key.expose(), "sk-raw-value");
}

// ---------------------------------------------------------------------------
// ApiKey::from_env
// ---------------------------------------------------------------------------

#[test]
fn api_key_from_env_missing_returns_missing_credential() {
    // Var name deliberately implausible so we don't care about host env state.
    let var = "ARCHON_AGS_701_SHOULD_NEVER_EXIST";
    let err = ApiKey::from_env(var).expect_err("must fail when env var is unset");
    match err {
        ProviderError::MissingCredential { var: got } => assert_eq!(got, var),
        other => panic!("expected MissingCredential, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// ProviderError: variants must exist and format sensibly
// ---------------------------------------------------------------------------

#[test]
fn provider_error_missing_credential_display() {
    let e = ProviderError::MissingCredential {
        var: "GROQ_API_KEY".into(),
    };
    let s = e.to_string();
    assert!(s.contains("GROQ_API_KEY"));
    assert!(s.to_lowercase().contains("missing"));
}

#[test]
fn provider_error_auth_failed_display() {
    let e = ProviderError::AuthFailed {
        name: "groq".into(),
        detail: "401 unauthorized".into(),
    };
    let s = e.to_string();
    assert!(s.contains("groq"));
    assert!(s.contains("401"));
}

#[test]
fn provider_error_unreachable_display() {
    let e = ProviderError::Unreachable {
        name: "ollama".into(),
        cause: "connection refused".into(),
    };
    let s = e.to_string();
    assert!(s.contains("ollama"));
    assert!(s.contains("connection refused"));
}

#[test]
fn provider_error_invalid_response_display() {
    let e = ProviderError::InvalidResponse {
        name: "openrouter".into(),
        detail: "missing 'choices' field".into(),
    };
    let s = e.to_string();
    assert!(s.contains("openrouter"));
    assert!(s.contains("choices"));
}

#[test]
fn provider_error_exhaustive_variants() {
    // Exhaustive match forces us to handle every variant. If a new one is
    // added, this test will fail to compile — forcing downstream tasks to
    // consider whether they need to handle it.
    let e = ProviderError::MissingCredential { var: "X".into() };
    match e {
        ProviderError::MissingCredential { .. } => {}
        ProviderError::AuthFailed { .. } => {}
        ProviderError::Unreachable { .. } => {}
        ProviderError::InvalidResponse { .. } => {}
        ProviderError::Http(_) => {}
    }
}

#[test]
fn provider_error_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ProviderError>();
}
