//! TASK-AGS-700 Gate 1: Tests-first for the `providers` scaffolding types.
//!
//! These tests pin the public shape of `ProviderDescriptor`, `ProviderFeatures`,
//! `AuthFlavor`, `CompatKind`, and `ProviderQuirks` before any downstream Phase 7
//! task (701..711) consumes them. They cover:
//!
//! - crate-root re-exports from `archon_llm::*`
//! - field counts and names (via exhaustive struct-literal construction)
//! - const helper behaviour on `ProviderFeatures`
//! - default derivation on `ProviderQuirks`
//! - serde round-trip for `ProviderFeatures` (proves Serialize + Deserialize)
//! - no secret leakage in `ProviderDescriptor` (NFR-SECURITY-001)

use std::collections::HashMap;

use archon_llm::{
    AuthFlavor, CompatKind, ProviderDescriptor, ProviderFeatures, ProviderQuirks,
};

// ---------------------------------------------------------------------------
// ProviderFeatures: shape + const helpers
// ---------------------------------------------------------------------------

#[test]
fn provider_features_none_is_all_false() {
    let f = ProviderFeatures::none();
    assert!(!f.streaming);
    assert!(!f.tool_calling);
    assert!(!f.vision);
    assert!(!f.embeddings);
    assert!(!f.json_mode);
}

#[test]
fn provider_features_chat_only_enables_streaming_only() {
    let f = ProviderFeatures::chat_only();
    assert!(f.streaming, "chat_only must support streaming");
    assert!(!f.tool_calling);
    assert!(!f.vision);
    assert!(!f.embeddings);
    assert!(!f.json_mode);
}

#[test]
fn provider_features_is_copy_and_eq() {
    // If this compiles, ProviderFeatures is Copy + PartialEq + Eq.
    let a = ProviderFeatures::none();
    let b = a; // Copy
    assert_eq!(a, b);
    assert_ne!(a, ProviderFeatures::chat_only());
}

#[test]
fn provider_features_has_exactly_five_bool_fields() {
    // Exhaustive struct literal — will fail to compile if fields added/removed/renamed.
    let f = ProviderFeatures {
        streaming: true,
        tool_calling: true,
        vision: true,
        embeddings: true,
        json_mode: true,
    };
    assert!(f.streaming);
    assert!(f.tool_calling);
    assert!(f.vision);
    assert!(f.embeddings);
    assert!(f.json_mode);
}

#[test]
fn provider_features_round_trips_through_json() {
    let f = ProviderFeatures {
        streaming: true,
        tool_calling: false,
        vision: true,
        embeddings: false,
        json_mode: true,
    };
    let s = serde_json::to_string(&f).expect("serialize");
    let back: ProviderFeatures = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(f, back);
}

// ---------------------------------------------------------------------------
// AuthFlavor + CompatKind: variant existence
// ---------------------------------------------------------------------------

#[test]
fn auth_flavor_variants_exist() {
    let _bearer: AuthFlavor = AuthFlavor::BearerApiKey;
    let _none: AuthFlavor = AuthFlavor::None;
    let _basic: AuthFlavor = AuthFlavor::BasicAuth;
    let _custom: AuthFlavor = AuthFlavor::Custom("x-api-token".to_string());
    // Pattern-match forces us to handle every variant — if one is added, this breaks.
    match AuthFlavor::BearerApiKey {
        AuthFlavor::BearerApiKey => {}
        AuthFlavor::None => {}
        AuthFlavor::BasicAuth => {}
        AuthFlavor::Custom(_) => {}
    }
}

#[test]
fn compat_kind_variants_exist() {
    let _a: CompatKind = CompatKind::OpenAiCompat;
    let _b: CompatKind = CompatKind::Native;
    match CompatKind::Native {
        CompatKind::OpenAiCompat => {}
        CompatKind::Native => {}
    }
}

// ---------------------------------------------------------------------------
// ProviderDescriptor: exhaustive field construction
// (TASK-AGS-700: 9 fields; TASK-AGS-705 added `quirks` -> 10 fields)
// ---------------------------------------------------------------------------

#[test]
fn provider_descriptor_exhaustive_construction() {
    let mut headers = HashMap::new();
    headers.insert("x-vendor".to_string(), "groq".to_string());

    let url = url::Url::parse("https://api.groq.com/openai/v1").unwrap();

    // Exhaustive struct literal — fails to compile if field names/types drift.
    let d = ProviderDescriptor {
        id: "groq".to_string(),
        display_name: "Groq".to_string(),
        base_url: url.clone(),
        auth_flavor: AuthFlavor::BearerApiKey,
        env_key_var: "GROQ_API_KEY".to_string(),
        compat_kind: CompatKind::OpenAiCompat,
        default_model: "llama-3.3-70b-versatile".to_string(),
        supports: ProviderFeatures::chat_only(),
        headers,
        quirks: ProviderQuirks::DEFAULT,
    };

    assert_eq!(d.id, "groq");
    assert_eq!(d.display_name, "Groq");
    assert_eq!(d.base_url, url);
    assert_eq!(d.env_key_var, "GROQ_API_KEY");
    assert_eq!(d.default_model, "llama-3.3-70b-versatile");
    assert!(d.supports.streaming);
    assert_eq!(d.headers.get("x-vendor").map(String::as_str), Some("groq"));
    assert!(matches!(d.auth_flavor, AuthFlavor::BearerApiKey));
    assert!(matches!(d.compat_kind, CompatKind::OpenAiCompat));
}

#[test]
fn provider_descriptor_is_clone() {
    let d = ProviderDescriptor {
        id: "a".into(),
        display_name: "A".into(),
        base_url: url::Url::parse("https://example.test/").unwrap(),
        auth_flavor: AuthFlavor::None,
        env_key_var: String::new(),
        compat_kind: CompatKind::Native,
        default_model: "m".into(),
        supports: ProviderFeatures::none(),
        headers: HashMap::new(),
        quirks: ProviderQuirks::DEFAULT,
    };
    let _cloned = d.clone(); // proves Clone is derived
}

// ---------------------------------------------------------------------------
// ProviderQuirks: DEFAULT baseline matches "no deviation" semantics
// (TASK-AGS-705 replaced Option/HashMap placeholders with enums +
// &'static [&'static str])
// ---------------------------------------------------------------------------

#[test]
fn provider_quirks_default_is_baseline() {
    use archon_llm::providers::{StreamDelimiter, ToolCallFormat};
    let q = ProviderQuirks::default();
    assert_eq!(q.tool_call_format, ToolCallFormat::Standard);
    assert_eq!(q.stream_delimiter, StreamDelimiter::Sse);
    assert!(q.ignore_response_fields.is_empty());
    // DEFAULT const matches Default::default()
    assert_eq!(q, ProviderQuirks::DEFAULT);
}

// ---------------------------------------------------------------------------
// NFR-SECURITY-001: descriptor never names a secret
// ---------------------------------------------------------------------------

/// The *descriptor* points at an env var (`env_key_var`) but MUST NOT carry
/// the secret itself. This compile-time + structural check is paired with the
/// grep in the task validation commands.
#[test]
fn descriptor_has_no_secret_field() {
    // If a field named `api_key` / `secret` is ever added, this test stays green,
    // but the `grep` in Gate 5 will flag it. This assertion documents intent.
    let d = ProviderDescriptor {
        id: "x".into(),
        display_name: "X".into(),
        base_url: url::Url::parse("https://example.test/").unwrap(),
        auth_flavor: AuthFlavor::BearerApiKey,
        env_key_var: "X_API_KEY".into(),
        compat_kind: CompatKind::OpenAiCompat,
        default_model: "m".into(),
        supports: ProviderFeatures::none(),
        headers: HashMap::new(),
        quirks: ProviderQuirks::DEFAULT,
    };
    // Prove the descriptor only *names* the env var, not the value.
    assert_eq!(d.env_key_var, "X_API_KEY");
}
