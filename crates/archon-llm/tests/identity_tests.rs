use std::collections::HashMap;

use archon_llm::identity::{
    DEFAULT_BETAS, IdentityMode, IdentityProvider, discover_betas_from_claude, resolve_betas,
};

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

    let blocks = provider.system_prompt_blocks("static content", "dynamic content");

    // Block 0: identity prefix (scope = org)
    assert_eq!(blocks[0]["cache_control"]["scope"].as_str(), Some("org"));

    // Block 1: static (scope = global)
    assert_eq!(blocks[1]["cache_control"]["scope"].as_str(), Some("global"));

    // Block 2: dynamic (no cache_control)
    assert!(blocks[2].get("cache_control").is_none());
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
