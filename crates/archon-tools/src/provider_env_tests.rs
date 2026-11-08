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

    let resolved =
        resolve_operator_provider_env(&policy, &policy.required_keys, &policy.profile_sources)
            .await;
    if assert_disabled_operator_grants(&resolved) {
        return;
    }

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

    let resolved =
        resolve_operator_provider_env(&policy, &policy.required_keys, &policy.profile_sources)
            .await;
    if assert_disabled_operator_grants(&resolved) {
        return;
    }

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

    let resolved =
        resolve_operator_provider_env(&policy, &policy.required_keys, &policy.profile_sources)
            .await;
    if assert_disabled_operator_grants(&resolved) {
        return;
    }

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

    let resolved =
        resolve_operator_provider_env(&policy, &policy.required_keys, &policy.profile_sources)
            .await;
    if assert_disabled_operator_grants(&resolved) {
        return;
    }

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

    let resolved =
        resolve_provider_env_with_timeout(&policy, Duration::from_millis(10), true).await;

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

/// Sourcing a profile must never go through the WSL launcher.
///
/// These three tests replace `profile_shell_prefers_discovered_sh`,
/// `profile_shell_uses_discovered_bash_when_sh_is_unavailable` and
/// `profile_shell_falls_back_to_path_sh`, which exercised a local
/// `select_profile_shell(sh, bash)` given hand-written paths. That helper is
/// gone: selection now lives in `archon-shell`, which is where the precedence
/// is tested against every candidate layout.
///
/// What those tests could not catch is the case that actually broke. They only
/// ever passed Git-style paths in, so they were green while the real lookup —
/// a bare `which("sh").or(which("bash"))` — resolved to
/// `C:\Windows\System32\bash.exe` on any machine without Git's `bin` on PATH.
/// Sourcing a Windows-path profile through WSL then failed, and callers saw
/// `ResolutionError` where they expected `Missing` (#118).
#[test]
fn profile_shell_is_never_the_wsl_launcher() {
    let shell = profile_shell();
    let parent = shell
        .parent()
        .and_then(|dir| dir.file_name())
        .and_then(|dir| dir.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let name = shell
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let is_launcher =
        name == "bash.exe" && ["system32", "sysnative", "windowsapps"].contains(&parent.as_str());
    assert!(
        !is_launcher,
        "profile sourcing resolved to the WSL launcher at {}",
        shell.display()
    );
}

/// And it must never be `cmd`, which would misread a POSIX profile script
/// rather than refuse it.
#[test]
fn profile_shell_is_a_posix_shell_not_cmd() {
    let shell = profile_shell();
    let name = shell
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert_ne!(name, "cmd", "profile sourcing must not run under cmd");
}

#[cfg(unix)]
#[tokio::test]
async fn profile_script_runs_under_posix_sh() {
    let dir = tempfile::tempdir().expect("tempdir");
    let profile = dir.path().join("profile");
    let mut file = std::fs::File::create(&profile).expect("profile");
    writeln!(file, "export ARCHON_TEST_POSIX_KEY=posix-value").unwrap();
    let keys = vec!["ARCHON_TEST_POSIX_KEY".to_string()];
    let profiles = vec![profile.display().to_string()];

    let output = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(profile_script(&keys, &profiles))
        .output()
        .await
        .expect("run POSIX shell");

    assert!(
        output.status.success(),
        "profile script must be POSIX-compatible: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        parse_profile_output(&output.stdout, &keys).expect("parse profile output"),
        BTreeMap::from([(
            "ARCHON_TEST_POSIX_KEY".to_string(),
            "posix-value".to_string()
        )])
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

#[tokio::test]
async fn authored_policy_cannot_execute_a_profile_or_grant_ambient_keys() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("executed");
    let profile = dir.path().join("profile");
    std::fs::write(
        &profile,
        format!(
            "touch '{}'\nexport ARCHON_R2_AUTHORED_KEY=synthetic\n",
            marker.display()
        ),
    )
    .unwrap();
    let mut policy = ProviderEnvPolicy::new(vec!["PATH".into(), "ARCHON_R2_AUTHORED_KEY".into()]);
    policy.profile_sources = vec![profile.display().to_string()];
    let resolution = resolve_provider_env(&policy).await;
    assert!(!marker.exists(), "authored profile must never execute");
    let mut env = Vec::new();
    resolution.apply_to_env(&mut env);
    assert!(
        env.is_empty(),
        "authored requirement must not confer ambient authority"
    );
}

#[tokio::test]
async fn authored_requirements_cannot_grant_ambient_environment() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("empty-profile");
    std::fs::write(&profile, "").unwrap();
    let mut policy = ProviderEnvPolicy::new(vec!["PATH".into()]);
    policy.profile_sources = vec![profile.display().to_string()];
    let resolution = resolve_provider_env(&policy).await;
    let mut env = Vec::new();
    resolution.apply_to_env(&mut env);
    assert!(
        env.is_empty(),
        "generated requirements must not authorize ambient variables"
    );
}

#[tokio::test]
async fn negative_mention_is_not_an_operator_grant() {
    let policy = ProviderEnvPolicy::new(vec!["X_API_KEY".into()]);
    let resolution =
        resolve_operator_provider_env(&policy, &["never expose X_API_KEY".into()], &[]).await;
    assert!(resolution.proof.redacted_env_keys_checked.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn structured_operator_grant_reaches_command_and_undeclared_key_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("profile");
    std::fs::write(&profile, "export DECLARED_API_KEY=synthetic-declared\nexport UNDECLARED_API_KEY=synthetic-undeclared\n").unwrap();
    let policy =
        ProviderEnvPolicy::new(vec!["DECLARED_API_KEY".into(), "UNDECLARED_API_KEY".into()]);
    let resolved = resolve_operator_provider_env(
        &policy,
        &["DECLARED_API_KEY".into()],
        &[profile.display().to_string()],
    )
    .await;
    let mut env = Vec::new();
    resolved.apply_to_env(&mut env);
    let status = archon_shell::execution_environment::command("/bin/sh")
        .0
        .envs(env)
        .args([
            "-c",
            if archon_shell::read_boundary::operator_grants_supported() {
                "test -n \"$DECLARED_API_KEY\" && test -z \"${UNDECLARED_API_KEY+x}\""
            } else {
                "test -z \"${DECLARED_API_KEY+x}\" && test -z \"${UNDECLARED_API_KEY+x}\""
            },
        ])
        .status()
        .unwrap();
    assert!(status.success());
}

#[tokio::test]
async fn operator_env_r5_uncontained_host_never_resolves_operator_grants() {
    if matches!(
        archon_shell::write_boundary::mechanism(),
        Ok(archon_shell::write_boundary::Mechanism::SandboxExec
            | archon_shell::write_boundary::Mechanism::Landlock { .. })
    ) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("provider-profile");
    std::fs::write(
        &profile,
        "export ARCHON_R5_PROVIDER_KEY=synthetic-r5-value\n",
    )
    .unwrap();
    let policy = ProviderEnvPolicy {
        required_keys: vec!["ARCHON_R5_PROVIDER_KEY".into()],
        profile_sources: vec![profile.display().to_string()],
        reason: None,
    };
    let result =
        resolve_operator_provider_env(&policy, &policy.required_keys, &policy.profile_sources)
            .await;
    assert!(
        result.env.is_empty(),
        "unsupported hosts must disable all operator grants"
    );
}

fn assert_disabled_operator_grants(resolved: &ProviderEnvResolution) -> bool {
    if archon_shell::read_boundary::operator_grants_supported() {
        return false;
    }
    assert!(resolved.env.is_empty());
    assert!(resolved.proof.profile_sources_checked.is_empty());
    assert!(
        resolved
            .proof
            .redacted_env_keys_checked
            .iter()
            .all(|key| key.state == ProviderEnvKeyState::Missing)
    );
    true
}
