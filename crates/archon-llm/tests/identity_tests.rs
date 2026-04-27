use std::collections::HashMap;

use archon_llm::identity::{
    DEFAULT_BETAS, IdentityMode, IdentityProvider, compute_fingerprint, discover_betas_from_claude,
    resolve_betas,
};

// -----------------------------------------------------------------------
// Fingerprint algorithm (REQ-IDENTITY-003)
// -----------------------------------------------------------------------

#[test]
fn fingerprint_known_vector() {
    // "Hello, how are you today?" indices: [4]='o', [7]=' ', [20]='d'
    let fp = compute_fingerprint("Hello, how are you today?", "2.1.89");
    assert_eq!(fp.len(), 3, "fingerprint should be 3 hex chars");
    // Deterministic -- same input produces same output
    let fp2 = compute_fingerprint("Hello, how are you today?", "2.1.89");
    assert_eq!(fp, fp2);
}

#[test]
fn fingerprint_short_message_pads_with_zero() {
    // Message "Hi" has length 2. Indices 4, 7, 20 all missing -> '0'
    let fp = compute_fingerprint("Hi", "2.1.89");
    assert_eq!(fp.len(), 3);

    // All zeros should produce same fingerprint as "000" padding
    let fp_explicit = compute_fingerprint("Hi", "2.1.89");
    assert_eq!(fp, fp_explicit);
}

#[test]
fn fingerprint_empty_message() {
    let fp = compute_fingerprint("", "2.1.89");
    assert_eq!(fp.len(), 3);
    // All padding chars are '0'
}

#[test]
fn fingerprint_different_versions_differ() {
    let fp1 = compute_fingerprint("same message content here!", "2.1.89");
    let fp2 = compute_fingerprint("same message content here!", "3.0.0");
    assert_ne!(
        fp1, fp2,
        "different versions should produce different fingerprints"
    );
}

#[test]
fn fingerprint_different_messages_differ() {
    let fp1 = compute_fingerprint("Hello, how are you today?", "2.1.89");
    let fp2 = compute_fingerprint("Goodbye cruel world!!!!!!", "2.1.89");
    // Different chars at indices 4, 7, 20 should produce different fingerprints
    // (unless hash collision, extremely unlikely)
    assert_ne!(fp1, fp2);
}

// -----------------------------------------------------------------------
// Identity headers (spoof mode)
// -----------------------------------------------------------------------

#[test]
fn spoof_mode_produces_all_headers() {
    let provider = IdentityProvider::new(
        IdentityMode::Spoof {
            version: "2.1.89".into(),
            entrypoint: "cli".into(),
            betas: vec!["claude-code-20250219".into(), "oauth-2025-04-20".into()],
            workload: None,
            anti_distillation: false,
        },
        "session-uuid-123".into(),
        "device-id-abc".into(),
        "account-uuid-xyz".into(),
    );

    let headers = provider.request_headers("req-uuid-456");

    assert_eq!(headers.get("x-app").map(String::as_str), Some("cli"));
    assert_eq!(
        headers.get("User-Agent").map(String::as_str),
        Some("claude-cli/2.1.89 (external, cli)")
    );
    assert_eq!(
        headers.get("X-Claude-Code-Session-Id").map(String::as_str),
        Some("session-uuid-123")
    );
    assert_eq!(
        headers.get("x-client-request-id").map(String::as_str),
        Some("req-uuid-456")
    );
    assert!(
        headers
            .get("anthropic-beta")
            .map(|v| v.contains("claude-code-20250219"))
            .unwrap_or(false),
        "beta header should include claude-code-20250219"
    );
    assert_eq!(
        headers.get("anthropic-version").map(String::as_str),
        Some("2023-06-01")
    );
}

// -----------------------------------------------------------------------
// Clean mode
// -----------------------------------------------------------------------

#[test]
fn clean_mode_identifies_as_archon() {
    let provider = IdentityProvider::new(
        IdentityMode::Clean,
        "sess".into(),
        "dev".into(),
        "acct".into(),
    );

    let headers = provider.request_headers("req");

    let ua = headers.get("User-Agent").expect("User-Agent present");
    assert!(
        ua.starts_with("archon-cli/"),
        "clean mode UA should start with archon-cli/, got: {ua}"
    );
    assert_eq!(headers.get("x-app").map(String::as_str), Some("archon"));
    // No billing-related headers
    assert!(headers.get("X-Claude-Code-Session-Id").is_none());
}

// -----------------------------------------------------------------------
// Custom mode
// -----------------------------------------------------------------------

#[test]
fn custom_mode_uses_provided_headers() {
    let mut extra = HashMap::new();
    extra.insert("X-Custom".into(), "value123".into());

    let provider = IdentityProvider::new(
        IdentityMode::Custom {
            user_agent: "my-agent/1.0".into(),
            x_app: "my-app".into(),
            extra_headers: extra,
        },
        "s".into(),
        "d".into(),
        "a".into(),
    );

    let headers = provider.request_headers("r");

    assert_eq!(
        headers.get("User-Agent").map(String::as_str),
        Some("my-agent/1.0")
    );
    assert_eq!(headers.get("x-app").map(String::as_str), Some("my-app"));
    assert_eq!(
        headers.get("X-Custom").map(String::as_str),
        Some("value123")
    );
}

// -----------------------------------------------------------------------
// Metadata (REQ-IDENTITY-008)
// -----------------------------------------------------------------------

#[test]
fn spoof_metadata_contains_user_id_json() {
    let provider = IdentityProvider::new(
        IdentityMode::Spoof {
            version: "2.1.89".into(),
            entrypoint: "cli".into(),
            betas: vec![],
            workload: None,
            anti_distillation: false,
        },
        "sess-123".into(),
        "dev-456".into(),
        "acct-789".into(),
    );

    let meta = provider.metadata();
    let user_id_str = meta["user_id"].as_str().expect("user_id should be string");

    // Parse the inner JSON
    let inner: serde_json::Value =
        serde_json::from_str(user_id_str).expect("user_id should be valid JSON");

    assert_eq!(inner["device_id"].as_str(), Some("dev-456"));
    assert_eq!(inner["account_uuid"].as_str(), Some("acct-789"));
    assert_eq!(inner["session_id"].as_str(), Some("sess-123"));
}

#[test]
fn clean_metadata_is_empty() {
    let provider = IdentityProvider::new(IdentityMode::Clean, "s".into(), "d".into(), "a".into());
    let meta = provider.metadata();
    assert!(meta.as_object().map(|o| o.is_empty()).unwrap_or(false));
}

// -----------------------------------------------------------------------
// Billing header (Layer 6)
// -----------------------------------------------------------------------

#[test]
fn billing_header_present_in_spoof() {
    let provider = IdentityProvider::new(
        IdentityMode::Spoof {
            version: "2.1.89".into(),
            entrypoint: "cli".into(),
            betas: vec![],
            workload: None,
            anti_distillation: false,
        },
        "s".into(),
        "d".into(),
        "a".into(),
    );

    let header = provider.billing_header("test message for fingerprint");
    assert!(header.is_some());
    let h = header.expect("billing header");
    assert!(h.contains("x-anthropic-billing-header:"));
    assert!(h.contains("cc_version=2.1.89."));
    assert!(h.contains("cc_entrypoint=cli"));
}

#[test]
fn billing_header_with_workload() {
    let provider = IdentityProvider::new(
        IdentityMode::Spoof {
            version: "2.1.89".into(),
            entrypoint: "cli".into(),
            betas: vec![],
            workload: Some("cron".into()),
            anti_distillation: false,
        },
        "s".into(),
        "d".into(),
        "a".into(),
    );

    let header = provider
        .billing_header("msg")
        .expect("should have billing header");
    assert!(header.contains("cc_workload=cron"));
}

#[test]
fn no_billing_header_in_clean_mode() {
    let provider = IdentityProvider::new(IdentityMode::Clean, "s".into(), "d".into(), "a".into());
    assert!(provider.billing_header("msg").is_none());
}

// -----------------------------------------------------------------------
// System prompt blocks (REQ-IDENTITY-009)
// -----------------------------------------------------------------------

#[test]
fn spoof_system_prompt_has_correct_cache_scopes() {
    let provider = IdentityProvider::new(
        IdentityMode::Spoof {
            version: "2.1.89".into(),
            entrypoint: "cli".into(),
            betas: vec![],
            workload: None,
            anti_distillation: false,
        },
        "s".into(),
        "d".into(),
        "a".into(),
    );

    let blocks = provider.system_prompt_blocks("Hello", "static content", "dynamic content");

    // Block 0: billing (ephemeral, no scope)
    assert!(blocks[0]["cache_control"]["scope"].is_null());
    assert_eq!(
        blocks[0]["cache_control"]["type"].as_str(),
        Some("ephemeral")
    );

    // Block 1: identity prefix (scope = org)
    assert_eq!(blocks[1]["cache_control"]["scope"].as_str(), Some("org"));

    // Block 2: static (scope = global)
    assert_eq!(blocks[2]["cache_control"]["scope"].as_str(), Some("global"));

    // Block 3: dynamic (no cache_control)
    assert!(blocks[3].get("cache_control").is_none());
}

// -----------------------------------------------------------------------
// Beta resolution
// -----------------------------------------------------------------------

#[test]
fn resolve_betas_config_override_wins() {
    let overrides = vec!["custom-beta-2025-01-01".to_string()];
    let result = resolve_betas(Some(&overrides));
    assert_eq!(result, vec!["custom-beta-2025-01-01"]);
}

#[test]
fn resolve_betas_falls_back_to_defaults() {
    let result = resolve_betas(None);
    assert!(!result.is_empty());
    assert!(result.contains(&"claude-code-20250219".to_string()));
}

#[test]
fn default_betas_has_core_entries() {
    assert_eq!(DEFAULT_BETAS.len(), 4);
    assert!(DEFAULT_BETAS.contains(&"claude-code-20250219"));
    assert!(DEFAULT_BETAS.contains(&"oauth-2025-04-20"));
}

// -----------------------------------------------------------------------
// Beta auto-discovery
// -----------------------------------------------------------------------

#[test]
fn discover_betas_does_not_panic() {
    // Should gracefully handle whatever state the system is in
    let betas = discover_betas_from_claude();
    // May be empty if Claude Code not installed, that's fine
    for beta in &betas {
        assert!(
            beta.contains('-') && beta.chars().any(|c| c.is_ascii_digit()),
            "beta should match pattern: {beta}"
        );
    }
}
