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
    assert_eq!(
        resolved.proof.redacted_env_keys_checked[0].found_in,
        ProviderEnvFoundIn::Profile
    );
    assert_eq!(resolved.proof.resolver.exit_status, Some(0));
    assert_eq!(
        resolved.proof.resolver.status,
        ProviderEnvResolverStatus::Succeeded
    );
    assert!(resolved.proof.resolver.stderr.is_empty());
    assert_eq!(resolved.proof.profile_provenance.len(), 1);
    assert!(resolved.proof.profile_provenance[0].exists);
    assert!(
        resolved.proof.profile_provenance[0]
            .modified_unix_ms
            .is_some()
    );
    assert!(
        resolved.proof.profile_provenance[0]
            .content_sha256
            .is_some()
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
    assert_eq!(
        resolved.proof.redacted_env_keys_checked[0].found_in,
        ProviderEnvFoundIn::None
    );
}

#[tokio::test]
async fn adjacent_literal_exports_both_resolve_with_profile_provenance() {
    let dir = tempfile::tempdir().expect("tempdir");
    let profile = dir.path().join("profile");
    let first_secret = "adjacent-alpha-secret";
    let second_secret = "adjacent-beta-secret";
    let mut file = std::fs::File::create(&profile).expect("profile");
    writeln!(file, "export ARCHON_TEST_ADJACENT_A={first_secret}").unwrap();
    writeln!(file, "export ARCHON_TEST_ADJACENT_B={second_secret}").unwrap();
    let policy = ProviderEnvPolicy {
        required_keys: vec![
            "ARCHON_TEST_ADJACENT_A".to_string(),
            "ARCHON_TEST_ADJACENT_B".to_string(),
        ],
        profile_sources: vec![profile.display().to_string()],
        reason: Some("adjacent exports regression".to_string()),
    };

    let resolved = resolve_provider_env(&policy).await;

    assert_eq!(
        resolved.proof.credential_state,
        ProviderEnvCredentialState::Present
    );
    assert!(
        resolved
            .proof
            .redacted_env_keys_checked
            .iter()
            .all(|key| key.state == ProviderEnvKeyState::Present
                && key.found_in == ProviderEnvFoundIn::Profile)
    );
    let serialized = serde_json::to_string(&resolved.proof).unwrap();
    assert!(!serialized.contains(first_secret));
    assert!(!serialized.contains(second_secret));
}

#[tokio::test]
async fn set_but_empty_key_is_present_empty_and_fails_closed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let profile = dir.path().join("profile");
    let mut file = std::fs::File::create(&profile).expect("profile");
    writeln!(file, "export ARCHON_TEST_EMPTY_A=x").unwrap();
    writeln!(file, "export ARCHON_TEST_EMPTY_B=").unwrap();
    let policy = ProviderEnvPolicy {
        required_keys: vec![
            "ARCHON_TEST_EMPTY_A".to_string(),
            "ARCHON_TEST_EMPTY_B".to_string(),
        ],
        profile_sources: vec![profile.display().to_string()],
        reason: Some("empty state regression".to_string()),
    };

    let resolved = resolve_provider_env(&policy).await;

    assert_eq!(
        resolved.proof.redacted_env_keys_checked[0].state,
        ProviderEnvKeyState::Present
    );
    assert_eq!(
        resolved.proof.redacted_env_keys_checked[1].state,
        ProviderEnvKeyState::PresentEmpty
    );
    assert_eq!(
        resolved.proof.credential_state,
        ProviderEnvCredentialState::Missing
    );
}

#[tokio::test]
async fn profile_timeout_is_resolution_error_not_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let profile = dir.path().join("profile");
    let mut file = std::fs::File::create(&profile).expect("profile");
    writeln!(file, "sleep 1").unwrap();
    writeln!(file, "export ARCHON_TEST_TIMEOUT_KEY=x").unwrap();
    let policy = ProviderEnvPolicy {
        required_keys: vec!["ARCHON_TEST_TIMEOUT_KEY".to_string()],
        profile_sources: vec![profile.display().to_string()],
        reason: Some("timeout regression".to_string()),
    };

    let resolved = resolve_provider_env_with_timeout(&policy, Duration::from_millis(10)).await;

    assert_eq!(
        resolved.proof.redacted_env_keys_checked[0].state,
        ProviderEnvKeyState::ResolutionError
    );
    assert_eq!(
        resolved.proof.redacted_env_keys_checked[0].found_in,
        ProviderEnvFoundIn::None
    );
    assert_eq!(resolved.proof.errors.len(), 1);
    assert!(resolved.proof.errors[0].contains("timed out"));
    assert_eq!(
        resolved.proof.resolver.status,
        ProviderEnvResolverStatus::TimedOut
    );
}

#[test]
fn resolver_status_defaults_for_legacy_redacted_proofs() {
    let proof: ProviderEnvResolverProof = serde_json::from_value(serde_json::json!({
        "exit_status": 0,
        "stderr": ""
    }))
    .expect("legacy resolver proof");

    assert_eq!(proof.status, ProviderEnvResolverStatus::NotNeeded);
}

#[test]
fn malformed_nul_output_is_an_explicit_resolution_error() {
    let error = parse_profile_output(
        b"ARCHON_TEST_PARSE_KEY\t1\tvalue",
        &["ARCHON_TEST_PARSE_KEY".to_string()],
    )
    .expect_err("unterminated record must fail");

    assert!(error.contains("malformed NUL output"));
}
