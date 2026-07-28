use super::*;

struct FakeLivePinger {
    outcome: std::result::Result<(), String>,
}

impl ProviderLivePinger for FakeLivePinger {
    fn ping(&self, _endpoint: &str) -> std::result::Result<(), String> {
        self.outcome.clone()
    }
}

fn render_args(args: &[&str]) -> String {
    let handler = crate::command::providers::ProvidersHandler;
    let (mut ctx, mut rx) = crate::command::test_support::make_bug_ctx();
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    crate::command::registry::CommandHandler::execute(&handler, &mut ctx, &args).unwrap();
    let events = crate::command::test_support::drain_tui_events(&mut rx);
    assert_eq!(events.len(), 1);
    match events.into_iter().next().unwrap() {
        archon_tui::app::TuiEvent::TextDelta(text) => text,
        other => panic!("expected TextDelta, got {other:?}"),
    }
}
#[test]
fn execute_doctor_reports_local_state_without_tokens() {
    let body = render_args(&["doctor"]);
    assert!(body.contains("Provider doctor (local checks only)"));
    assert!(body.contains("Credentials file:"));
    assert!(body.contains("Anthropic OAuth:"));
    assert!(body.contains("Codex OAuth:"));
    assert!(!body.contains("accessToken"));
    assert!(!body.contains("refreshToken"));
}

#[test]
fn execute_doctor_live_reports_endpoint_checks() {
    let body = render_args(&["doctor", "--live"]);
    assert!(body.contains("Provider doctor (local checks + live endpoint reachability)"));
    assert!(body.contains("Live provider pings:"));
    assert!(!body.contains("accessToken"));
    assert!(!body.contains("refreshToken"));
}

#[test]
fn render_provider_doctor_from_json_redacts_credentials() {
    let future = chrono::Utc::now() + chrono::Duration::hours(1);
    let json = serde_json::json!({
        "claudeAiOauth": {
            "accessToken": "secret-anthropic-access",
            "refreshToken": "secret-anthropic-refresh",
            "expiresAt": future.timestamp_millis(),
            "scopes": ["user:inference"],
            "subscriptionType": "pro"
        },
        "openaiCodexOauth": {
            "accessToken": "secret-codex-access",
            "refreshToken": "secret-codex-refresh",
            "expiresAt": future.timestamp_millis(),
            "accountId": "acct_secret"
        }
    })
    .to_string();

    let body = render_provider_doctor_from_json(true, Some(&json), false);
    assert!(body.contains("Anthropic OAuth:  present"));
    assert!(body.contains("Codex OAuth:     present"));
    assert!(body.contains("Live provider pings: not requested"));
    assert!(!body.contains("secret-anthropic-access"));
    assert!(!body.contains("secret-codex-access"));
    assert!(!body.contains("acct_secret"));
}

#[test]
fn render_provider_doctor_live_uses_pinger_without_printing_tokens() {
    let future = chrono::Utc::now() + chrono::Duration::hours(1);
    let json = serde_json::json!({
        "claudeAiOauth": {
            "accessToken": "secret-anthropic-access",
            "refreshToken": "secret-anthropic-refresh",
            "expiresAt": future.timestamp_millis(),
            "scopes": ["user:inference"],
            "subscriptionType": "pro"
        },
        "openaiCodexOauth": {
            "accessToken": "secret-codex-access",
            "refreshToken": "secret-codex-refresh",
            "expiresAt": future.timestamp_millis(),
            "accountId": "acct_secret"
        }
    })
    .to_string();

    let pinger = FakeLivePinger { outcome: Ok(()) };
    let body = render_provider_doctor_with_pinger(
        true,
        Some(&json),
        None,
        false,
        true,
        ProviderDoctorEnv::default(),
        &pinger,
    );
    assert!(body.contains("Anthropic ok: endpoint reachable"));
    assert!(body.contains("Codex     ok: endpoint reachable"));
    assert!(!body.contains("secret-anthropic-access"));
    assert!(!body.contains("secret-codex-access"));
    assert!(!body.contains("acct_secret"));
}

#[test]
fn render_provider_doctor_live_skips_missing_or_disabled_credentials() {
    let pinger = FakeLivePinger { outcome: Ok(()) };
    let body = render_provider_doctor_with_pinger(
        false,
        None,
        None,
        true,
        true,
        ProviderDoctorEnv::default(),
        &pinger,
    );
    assert!(body.contains("Anthropic skipped: credentials missing"));
    assert!(body.contains("Codex     skipped: disabled by ARCHON_CODEX_DISABLED"));
}

#[test]
fn render_provider_doctor_uses_codex_cli_fallback_status() {
    let pinger = FakeLivePinger { outcome: Ok(()) };
    let body = render_provider_doctor_with_pinger(
        false,
        None,
        Some("present"),
        false,
        false,
        ProviderDoctorEnv::default(),
        &pinger,
    );
    assert!(body.contains("Codex OAuth:     present"));
    assert!(body.contains(
        "Codex spoof identity: loaded from bundled/config/env spoof identity at runtime"
    ));
    assert!(!body.contains("accessToken"));
    assert!(!body.contains("refreshToken"));
}

#[test]
fn render_provider_doctor_live_reports_ping_failure() {
    let future = chrono::Utc::now() + chrono::Duration::hours(1);
    let json = serde_json::json!({
        "claudeAiOauth": {
            "accessToken": "secret-anthropic-access",
            "refreshToken": "secret-anthropic-refresh",
            "expiresAt": future.timestamp_millis(),
            "scopes": ["user:inference"],
            "subscriptionType": "pro"
        }
    })
    .to_string();

    let pinger = FakeLivePinger {
        outcome: Err("synthetic failure".to_string()),
    };
    let body = render_provider_doctor_with_pinger(
        true,
        Some(&json),
        None,
        false,
        true,
        ProviderDoctorEnv::default(),
        &pinger,
    );
    assert!(body.contains("Anthropic failed: endpoint unreachable"));
    assert!(body.contains("synthetic failure"));
    assert!(body.contains("Codex     skipped: credentials missing"));
    assert!(!body.contains("secret-anthropic-access"));
}

#[test]
fn render_provider_doctor_reports_spoof_proxy_and_remediation() {
    let pinger = FakeLivePinger { outcome: Ok(()) };
    let env = ProviderDoctorEnv {
        anthropic_env_kind: EnvAnthropicCredentialKind::OAuthToken,
        anthropic_base_url_set: true,
        proxy_env_set: true,
    };
    let body = render_provider_doctor_with_pinger(false, None, None, false, false, env, &pinger);
    assert!(body.contains("ANTHROPIC_API_KEY env: OAuth token shaped"));
    assert!(body.contains("Anthropic base URL: custom via ANTHROPIC_BASE_URL"));
    assert!(body.contains("Proxy env:       set"));
    assert!(body.contains("Anthropic spoof identity: active for OAuth-shaped ANTHROPIC_API_KEY"));
    assert!(body.contains("Codex missing: run `archon auth login --provider openai-codex`"));
    assert!(!body.contains("sk-ant-oat"));
}

#[test]
fn render_provider_doctor_marks_codex_kill_switch() {
    let body = render_provider_doctor_from_json(false, None, true);
    assert!(body.contains("Codex OAuth:     disabled by ARCHON_CODEX_DISABLED"));
}
