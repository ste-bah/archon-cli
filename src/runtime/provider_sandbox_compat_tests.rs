use archon_core::config::ArchonConfig;
use archon_core::sandbox::OpenShellConfig;
use archon_llm::auth::{AuthProvider, OAuthCredentials};
use archon_llm::identity::{DEFAULT_BETAS, IdentityMode, IdentityProvider, resolve_identity_mode};
use archon_llm::types::Secret;
use chrono::{Duration, Utc};

#[test]
fn openshell_sandbox_does_not_change_anthropic_spoof_contract() {
    let mut config = ArchonConfig::default();
    config.sandbox.backend = "openshell".into();
    config.sandbox.openshell = OpenShellConfig {
        enabled: true,
        gateway: Some("team-gateway".into()),
        provider_injection: false,
        host_shell_fallback: false,
        ..OpenShellConfig::default()
    };
    let auth = AuthProvider::OAuthToken(OAuthCredentials {
        access_token: Secret::new("sk-ant-oat-test".to_string()),
        refresh_token: Secret::new("refresh".to_string()),
        expires_at: Utc::now() + Duration::hours(1),
        scopes: vec!["user".to_string()],
        subscription_type: "pro".to_string(),
    });

    let mode = resolve_identity_mode(&auth, false, &config.identity.as_view());
    let provider = IdentityProvider::new(
        mode,
        "session-123".into(),
        "device-456".into(),
        "account-789".into(),
    );
    let headers = provider.request_headers("request-abc");
    let metadata = provider.metadata();

    assert_eq!(config.sandbox.backend, "openshell");
    assert!(!config.sandbox.openshell.provider_injection);
    assert!(!config.sandbox.openshell.host_shell_fallback);
    assert!(matches!(provider.mode, IdentityMode::Spoof { .. }));
    assert_eq!(headers.get("x-app").map(String::as_str), Some("cli"));
    assert_eq!(
        headers.get("User-Agent").map(String::as_str),
        Some("claude-cli/2.1.89 (external, cli)")
    );
    assert_eq!(
        headers.get("X-Claude-Code-Session-Id").map(String::as_str),
        Some("session-123")
    );
    assert_eq!(
        headers.get("x-client-request-id").map(String::as_str),
        Some("request-abc")
    );
    assert_eq!(
        headers.get("anthropic-version").map(String::as_str),
        Some("2023-06-01")
    );
    for beta in DEFAULT_BETAS {
        assert!(
            headers
                .get("anthropic-beta")
                .is_some_and(|value| value.split(',').any(|observed| observed == *beta)),
            "missing spoof beta {beta}"
        );
    }
    let user_id: serde_json::Value =
        serde_json::from_str(metadata["user_id"].as_str().unwrap_or_default()).unwrap();
    assert_eq!(user_id["device_id"], "device-456");
    assert_eq!(user_id["account_uuid"], "account-789");
    assert_eq!(user_id["session_id"], "session-123");
}
