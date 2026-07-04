use std::io::Write;

use super::*;

#[tokio::test]
async fn profile_preflight_reports_present_key_without_value() {
    let dir = tempfile::tempdir().expect("tempdir");
    let profile = dir.path().join("profile");
    let secret = "secret-from-profile-123";
    let mut file = std::fs::File::create(&profile).expect("profile");
    writeln!(file, "export ARCHON_TEST_PROVIDER_KEY={secret}").unwrap();
    let policy = ProviderEnvPolicy {
        required_keys: vec!["ARCHON_TEST_PROVIDER_KEY".to_string()],
        profile_sources: vec![profile.display().to_string()],
        reason: Some("test".to_string()),
    };

    let resolved = resolve_provider_env(&policy).await;

    assert_eq!(
        resolved.proof.credential_state,
        ProviderEnvCredentialState::Present
    );
    assert_eq!(
        resolved.proof.redacted_env_keys_checked[0].state,
        ProviderEnvKeyState::Present
    );
    let proof = serde_json::to_string(&resolved.proof).unwrap();
    assert!(!proof.contains(secret));
    assert_eq!(
        resolved.redact_text(&format!("value={secret}")),
        "value=<redacted:ARCHON_TEST_PROVIDER_KEY>"
    );
}

#[tokio::test]
async fn missing_key_is_reported_without_secret() {
    let policy = ProviderEnvPolicy {
        required_keys: vec!["ARCHON_TEST_MISSING_KEY".to_string()],
        profile_sources: vec!["/no/such/profile".to_string()],
        reason: None,
    };

    let resolved = resolve_provider_env(&policy).await;

    assert_eq!(
        resolved.proof.credential_state,
        ProviderEnvCredentialState::Missing
    );
    assert_eq!(
        resolved.proof.redacted_env_keys_checked[0].state,
        ProviderEnvKeyState::Missing
    );
}
