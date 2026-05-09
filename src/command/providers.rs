//! TASK-#210 SLASH-PROVIDERS — `/providers` slash-command handler.
//!
//! Lists every LLM provider registered in the workspace (37 total =
//! 6 native + 31 OpenAI-compatible) by reading the static
//! `archon_llm::providers::{list_native, list_compat}` registries.
//! No session state is touched — both registries are
//! `lazy_static`-style readonly statics, so the handler runs entirely
//! synchronously without populating a `CommandContext` snapshot.
//!
//! GHOST-003: 4 stub native providers (azure, cohere, copilot, minimax)
//! were removed — they returned LlmError::Unsupported with no real wire
//! implementations. The registry now has 6 real native entries.
//!
//! Output is a single `TuiEvent::TextDelta` carrying a two-section
//! aligned table (NATIVE then OPENAI-COMPAT) — matches the
//! `/status` / `/usage` / `/extra-usage` text-delta precedent rather
//! than the `/mcp` overlay pattern (the provider list is static and
//! does not warrant a custom `TuiEvent` variant + TUI overlay).

use anyhow::Result;
use chrono::Utc;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use archon_llm::providers::{
    CompatKind, ProviderDescriptor, ProviderFeatures, count_compat, count_native, list_compat,
    list_native, render_capability_table,
};

use crate::cli_args::ProvidersAction;

pub(crate) use crate::command::providers_slash::ProvidersHandler;

pub(crate) fn handle_providers(
    action: Option<ProvidersAction>,
    config: &archon_core::config::ArchonConfig,
) -> Result<()> {
    match action.unwrap_or(ProvidersAction::List) {
        ProvidersAction::List => print!("{}", render_provider_registry()),
        ProvidersAction::Capabilities => print!("{}", render_capability_table()),
        ProvidersAction::Status { provider, json } => print!(
            "{}",
            crate::command::providers_status::render_and_persist_provider_status(
                provider.as_deref(),
                config,
                json,
            )?
        ),
        ProvidersAction::Report { provider, json } => print!(
            "{}",
            crate::command::providers_health_report::render_provider_health_report(
                provider.as_deref(),
                config,
                json,
            )?
        ),
        ProvidersAction::Limits { provider } => print!(
            "{}",
            crate::command::providers_store_cli::render_provider_limits(provider.as_deref())?
        ),
        ProvidersAction::Profiles { action } => match action {
            crate::cli_args::ProviderProfilesAction::Import => {
                print!(
                    "{}",
                    crate::command::providers_profile_import::import_provider_profiles()?
                )
            }
            crate::cli_args::ProviderProfilesAction::List { provider } => print!(
                "{}",
                crate::command::providers_store_cli::render_provider_profiles(provider.as_deref())?
            ),
            crate::cli_args::ProviderProfilesAction::Inspect { profile_id } => print!(
                "{}",
                crate::command::providers_store_cli::render_provider_profile_inspect(&profile_id)?
            ),
            crate::cli_args::ProviderProfilesAction::CooldownClear { profile_id } => print!(
                "{}",
                crate::command::providers_store_cli::clear_provider_profile_cooldown(&profile_id)?
            ),
            crate::cli_args::ProviderProfilesAction::Select {
                provider,
                auth_kinds,
                preferred,
            } => print!(
                "{}",
                crate::command::providers_store_cli::render_provider_profile_selection(
                    &provider,
                    &auth_kinds,
                    preferred.as_deref(),
                )?
            ),
        },
        ProvidersAction::Doctor { live } => print!("{}", render_provider_doctor(live)),
    }
    Ok(())
}

pub(crate) fn render_provider_registry() -> String {
    let native = list_native();
    let compat = list_compat();
    let total = count_native() + count_compat();

    let mut out = String::with_capacity(4096);
    out.push('\n');
    out.push_str(&format!(
        "LLM provider registry ({total} total: {n_native} native + {n_compat} openai-compat)\n",
        total = total,
        n_native = count_native(),
        n_compat = count_compat(),
    ));
    out.push('\n');

    // NATIVE section.
    out.push_str(&format!("NATIVE ({})\n", count_native()));
    out.push_str(&header_row());
    out.push_str(&divider_row());
    for d in &native {
        debug_assert_eq!(d.compat_kind, CompatKind::Native);
        out.push_str(&fmt_provider_row(d));
    }
    out.push('\n');

    // OPENAI-COMPAT section.
    out.push_str(&format!("OPENAI-COMPAT ({})\n", count_compat()));
    out.push_str(&header_row());
    out.push_str(&divider_row());
    for d in &compat {
        debug_assert_eq!(d.compat_kind, CompatKind::OpenAiCompat);
        out.push_str(&fmt_provider_row(d));
    }
    out.push('\n');

    out.push_str(
        "Tip: configure a provider in [llm.<id>] in archon.toml; switch the active\n\
             model with /model <name>.\n",
    );
    out
}

pub(crate) fn render_provider_doctor(live: bool) -> String {
    let path = archon_llm::tokens::credentials_path();
    let credentials_json = std::fs::read_to_string(&path).ok();
    let codex_status = codex_status_from_disk(&path);
    let mut out = render_provider_doctor_with_pinger(
        path.exists(),
        credentials_json.as_deref(),
        codex_status,
        codex_disabled(),
        live,
        local_provider_env(),
        &TcpProviderLivePinger,
    );
    append_vlm_doctor(&mut out, live);
    out
}

#[cfg(test)]
fn render_provider_doctor_from_json(
    credentials_file_exists: bool,
    credentials_json: Option<&str>,
    codex_disabled: bool,
) -> String {
    render_provider_doctor_with_pinger(
        credentials_file_exists,
        credentials_json,
        None,
        codex_disabled,
        false,
        ProviderDoctorEnv::default(),
        &DisabledLivePinger,
    )
}

fn render_provider_doctor_with_pinger(
    credentials_file_exists: bool,
    credentials_json: Option<&str>,
    codex_status_override: Option<&'static str>,
    codex_disabled: bool,
    live: bool,
    env: ProviderDoctorEnv,
    pinger: &dyn ProviderLivePinger,
) -> String {
    let anthropic = credentials_json
        .and_then(|json| archon_llm::auth::parse_credentials_json(json).ok())
        .map(|creds| credential_status(creds.expires_at.timestamp_millis()));
    let codex = credentials_json
        .and_then(|json| archon_llm::auth::parse_codex_credentials_json(json).ok())
        .map(|creds| credential_status(creds.expires_at.timestamp_millis()))
        .or(codex_status_override);

    let mut out = String::new();
    if live {
        out.push_str("Provider doctor (local checks + live endpoint reachability)\n\n");
    } else {
        out.push_str("Provider doctor (local checks only)\n\n");
    }
    out.push_str(&format!(
        "Credentials file: {}\n",
        if credentials_file_exists {
            "present"
        } else {
            "missing"
        }
    ));
    out.push_str(&format!(
        "Anthropic OAuth:  {}\n",
        anthropic.unwrap_or("missing")
    ));
    let codex_status = if codex_disabled {
        "disabled by ARCHON_CODEX_DISABLED"
    } else {
        codex.unwrap_or("missing")
    };
    out.push_str(&format!("Codex OAuth:     {codex_status}\n"));
    out.push_str(&format!(
        "ANTHROPIC_API_KEY env: {}\n",
        env.anthropic_env_kind.as_str()
    ));
    out.push_str(&format!(
        "Anthropic base URL: {}\n",
        if env.anthropic_base_url_set {
            "custom via ANTHROPIC_BASE_URL"
        } else {
            "default"
        }
    ));
    out.push_str(&format!(
        "Proxy env:       {}\n",
        if env.proxy_env_set { "set" } else { "unset" }
    ));
    out.push_str(&format!(
        "Anthropic spoof identity: {}\n",
        anthropic_spoof_status(anthropic, env.anthropic_env_kind)
    ));
    out.push_str(&format!(
        "Codex spoof identity: {}\n",
        codex_spoof_status(codex, codex_disabled)
    ));
    out.push('\n');
    out.push_str("Capability source of truth: `archon providers capabilities` or `/providers capabilities`\n");
    render_live_provider_pings(&mut out, live, anthropic, codex, codex_disabled, pinger);
    render_remediation_hints(&mut out, anthropic, codex, codex_disabled, env);
    out
}

fn codex_status_from_disk(path: &std::path::Path) -> Option<&'static str> {
    archon_llm::tokens_codex::read_codex_credentials_locked(path)
        .ok()
        .map(|(creds, _mtime)| credential_status(creds.expires_at.timestamp_millis()))
}

fn append_vlm_doctor(out: &mut String, live: bool) {
    let policy = std::env::current_dir()
        .ok()
        .and_then(|cwd| archon_policy::load_effective_policy(&cwd).ok())
        .unwrap_or_default();
    let (provider, model) = archon_docs::vlm::factory::default_provider_summary(&policy);
    if !live {
        out.push_str(&format!(
            "VLM provider:   configured provider={} model={} (pass --live for health check)\n",
            provider,
            if model.is_empty() {
                "n/a"
            } else {
                model.as_str()
            }
        ));
        out.push_str(&format!(
            "PDF images:     pdfimages {}\n",
            pdfimages_doctor_status()
        ));
        return;
    }
    let report = archon_docs::vlm::factory::diagnostic_report(&policy);
    let line = match report.status {
        archon_docs::vlm::factory::VlmProviderInitStatus::Registered => {
            format!("ok — {}/{}", report.provider, report.model)
        }
        archon_docs::vlm::factory::VlmProviderInitStatus::Disabled => {
            format!("disabled — {}", report.message)
        }
        archon_docs::vlm::factory::VlmProviderInitStatus::Skipped => {
            format!(
                "skipped — {}/{}: {}",
                report.provider, report.model, report.message
            )
        }
    };
    out.push_str(&format!("VLM provider:   {line}\n"));
    out.push_str(&format!(
        "PDF images:     pdfimages {}\n",
        pdfimages_doctor_status()
    ));
}

fn pdfimages_doctor_status() -> String {
    let bin = std::env::var_os("ARCHON_PDFIMAGES_BIN").unwrap_or_else(|| "pdfimages".into());
    let display = std::path::PathBuf::from(&bin).display().to_string();
    match std::process::Command::new(&bin).arg("-v").output() {
        Ok(output) if output.status.success() || !output.stderr.is_empty() => {
            format!("ok — {display}")
        }
        Ok(output) => format!("unhealthy — {display} status={:?}", output.status.code()),
        Err(e) => format!("missing — {display} ({e})"),
    }
}

#[derive(Debug, Clone, Copy)]
struct ProviderDoctorEnv {
    anthropic_env_kind: EnvAnthropicCredentialKind,
    anthropic_base_url_set: bool,
    proxy_env_set: bool,
}

impl Default for ProviderDoctorEnv {
    fn default() -> Self {
        Self {
            anthropic_env_kind: EnvAnthropicCredentialKind::Missing,
            anthropic_base_url_set: false,
            proxy_env_set: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvAnthropicCredentialKind {
    Missing,
    ApiKey,
    OAuthToken,
    Unknown,
}

impl EnvAnthropicCredentialKind {
    fn as_str(self) -> &'static str {
        match self {
            EnvAnthropicCredentialKind::Missing => "missing",
            EnvAnthropicCredentialKind::ApiKey => "api key shaped",
            EnvAnthropicCredentialKind::OAuthToken => "OAuth token shaped",
            EnvAnthropicCredentialKind::Unknown => "set but unrecognized shape",
        }
    }
}

fn local_provider_env() -> ProviderDoctorEnv {
    ProviderDoctorEnv {
        anthropic_env_kind: std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .map(
                |value| match archon_llm::auth::classify_anthropic_credential(&value) {
                    archon_llm::auth::AnthropicCredentialKind::Absent => {
                        EnvAnthropicCredentialKind::Missing
                    }
                    archon_llm::auth::AnthropicCredentialKind::ApiKey => {
                        EnvAnthropicCredentialKind::ApiKey
                    }
                    archon_llm::auth::AnthropicCredentialKind::OAuthToken => {
                        EnvAnthropicCredentialKind::OAuthToken
                    }
                    archon_llm::auth::AnthropicCredentialKind::Unknown => {
                        EnvAnthropicCredentialKind::Unknown
                    }
                },
            )
            .unwrap_or(EnvAnthropicCredentialKind::Missing),
        anthropic_base_url_set: std::env::var("ANTHROPIC_BASE_URL")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false),
        proxy_env_set: ["HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY"]
            .iter()
            .any(|key| {
                std::env::var(key)
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false)
            }),
    }
}

fn anthropic_spoof_status(
    anthropic_file_status: Option<&'static str>,
    env_kind: EnvAnthropicCredentialKind,
) -> &'static str {
    if matches!(env_kind, EnvAnthropicCredentialKind::OAuthToken) {
        "active for OAuth-shaped ANTHROPIC_API_KEY"
    } else if matches!(anthropic_file_status, Some("present")) {
        "active for Claude OAuth credential file"
    } else {
        "not required unless Anthropic OAuth is used"
    }
}

fn codex_spoof_status(codex_status: Option<&'static str>, codex_disabled: bool) -> &'static str {
    if codex_disabled {
        "disabled by ARCHON_CODEX_DISABLED"
    } else if matches!(codex_status, Some("present")) {
        "loaded from bundled/config/env spoof identity at runtime"
    } else {
        "unavailable until Codex OAuth credentials are present"
    }
}

fn render_remediation_hints(
    out: &mut String,
    anthropic: Option<&'static str>,
    codex: Option<&'static str>,
    codex_disabled: bool,
    env: ProviderDoctorEnv,
) {
    out.push_str("Remediation:\n");
    if anthropic.is_none() && env.anthropic_env_kind == EnvAnthropicCredentialKind::Missing {
        out.push_str("  - Anthropic missing: run `archon auth login --provider anthropic` or set ANTHROPIC_API_KEY.\n");
    }
    if matches!(anthropic, Some("present but expired")) {
        out.push_str("  - Anthropic expired: run `archon auth login --provider anthropic` to refresh credentials.\n");
    }
    if env.anthropic_env_kind == EnvAnthropicCredentialKind::Unknown {
        out.push_str("  - ANTHROPIC_API_KEY shape is unknown: use sk-ant-api... for API keys or sk-ant-oat... for OAuth spoofing.\n");
    }
    if codex_disabled {
        out.push_str("  - Codex disabled: unset ARCHON_CODEX_DISABLED to enable Codex surfaces.\n");
    } else if codex.is_none() {
        out.push_str("  - Codex missing: run `archon auth login --provider openai-codex` for Codex TUI/chat support.\n");
    } else if matches!(codex, Some("present but expired")) {
        out.push_str("  - Codex expired: run `archon auth login --provider openai-codex` to refresh credentials.\n");
    }
    out.push_str("  - Capability mismatch: run `archon providers capabilities` before using a provider on pipelines/subagents.\n");
}

fn render_live_provider_pings(
    out: &mut String,
    live: bool,
    anthropic: Option<&'static str>,
    codex: Option<&'static str>,
    codex_disabled: bool,
    pinger: &dyn ProviderLivePinger,
) {
    if !live {
        out.push_str(
            "Live provider pings: not requested (pass --live to enable opt-in endpoint checks).\n",
        );
        return;
    }

    out.push_str("Live provider pings:\n");
    render_live_ping_row(
        out,
        "Anthropic",
        "api.anthropic.com:443",
        anthropic,
        false,
        pinger,
    );
    render_live_ping_row(
        out,
        "Codex",
        "chatgpt.com:443",
        codex,
        codex_disabled,
        pinger,
    );
}

fn render_live_ping_row(
    out: &mut String,
    label: &str,
    endpoint: &str,
    credential: Option<&'static str>,
    disabled: bool,
    pinger: &dyn ProviderLivePinger,
) {
    let status = if disabled {
        "skipped: disabled by ARCHON_CODEX_DISABLED".to_string()
    } else {
        match credential {
            None => "skipped: credentials missing".to_string(),
            Some("present but expired") => "skipped: credentials expired".to_string(),
            Some(_) => match pinger.ping(endpoint) {
                Ok(()) => format!("ok: endpoint reachable ({endpoint})"),
                Err(err) => format!("failed: endpoint unreachable ({endpoint}: {err})"),
            },
        }
    };
    out.push_str(&format!("  {label:<9} {status}\n"));
}

trait ProviderLivePinger {
    fn ping(&self, endpoint: &str) -> std::result::Result<(), String>;
}

#[cfg(test)]
struct DisabledLivePinger;

#[cfg(test)]
impl ProviderLivePinger for DisabledLivePinger {
    fn ping(&self, _endpoint: &str) -> std::result::Result<(), String> {
        Ok(())
    }
}

struct TcpProviderLivePinger;

impl ProviderLivePinger for TcpProviderLivePinger {
    fn ping(&self, endpoint: &str) -> std::result::Result<(), String> {
        let mut addrs = endpoint
            .to_socket_addrs()
            .map_err(|err| format!("resolve failed: {err}"))?;
        let addr = addrs
            .next()
            .ok_or_else(|| "no socket address".to_string())?;
        TcpStream::connect_timeout(&addr, Duration::from_millis(1_500))
            .map(|_| ())
            .map_err(|err| err.to_string())
    }
}

fn credential_status(expires_at_ms: i64) -> &'static str {
    let now_ms = Utc::now().timestamp_millis();
    if expires_at_ms <= now_ms {
        "present but expired"
    } else {
        "present"
    }
}

fn codex_disabled() -> bool {
    std::env::var("ARCHON_CODEX_DISABLED")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

// Column widths kept in module-private constants so the header,
// divider, and data rows stay in lockstep — change one, change all.
const COL_ID: usize = 15;
const COL_DISPLAY: usize = 20;
const COL_MODEL: usize = 36;

fn header_row() -> String {
    format!(
        "  {:<id$}  {:<display$}  {:<model$}  features\n",
        "id",
        "display name",
        "default model",
        id = COL_ID,
        display = COL_DISPLAY,
        model = COL_MODEL,
    )
}

fn divider_row() -> String {
    let make = |n: usize| "-".repeat(n);
    format!(
        "  {}  {}  {}  {}\n",
        make(COL_ID),
        make(COL_DISPLAY),
        make(COL_MODEL),
        make(8),
    )
}

fn fmt_provider_row(d: &ProviderDescriptor) -> String {
    let display = truncate_chars(&d.display_name, COL_DISPLAY);
    let model = truncate_chars(&d.default_model, COL_MODEL);
    let mut features = fmt_features(&d.supports);
    if d.is_gap {
        features.push_str(" [gap]");
    }
    format!(
        "  {:<id$}  {:<display$}  {:<model$}  {}\n",
        d.id,
        display,
        model,
        features,
        id = COL_ID,
        display = COL_DISPLAY,
        model = COL_MODEL,
    )
}

/// Truncate a `&str` to at most `max` Unicode characters, appending
/// `…` when shortened. Char-aware — never panics on multi-byte input.
fn truncate_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    // max - 1 chars plus a `…` (1 char) keeps total <= max.
    let take = max.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}

fn fmt_features(f: &ProviderFeatures) -> String {
    let mut parts: Vec<&'static str> = Vec::with_capacity(5);
    if f.streaming {
        parts.push("stream");
    }
    if f.tool_calling {
        parts.push("tools");
    }
    if f.vision {
        parts.push("vision");
    }
    if f.embeddings {
        parts.push("embed");
    }
    if f.json_mode {
        parts.push("json");
    }
    if parts.is_empty() {
        "(none)".to_string()
    } else {
        parts.join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::CommandHandler;
    use crate::command::test_support::*;
    use archon_tui::app::TuiEvent;

    struct FakeLivePinger {
        outcome: std::result::Result<(), String>,
    }

    impl ProviderLivePinger for FakeLivePinger {
        fn ping(&self, _endpoint: &str) -> std::result::Result<(), String> {
            self.outcome.clone()
        }
    }

    fn render() -> String {
        let handler = ProvidersHandler;
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match events.into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    fn render_args(args: &[&str]) -> String {
        let handler = ProvidersHandler;
        let (mut ctx, mut rx) = make_bug_ctx();
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        handler.execute(&mut ctx, &args).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match events.into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn execute_emits_total_count_line() {
        let body = render();
        assert!(
            body.contains("37 total: 6 native + 31 openai-compat"),
            "totals line missing or wrong; body:\n{}",
            body
        );
    }

    #[test]
    fn execute_lists_both_section_headers() {
        let body = render();
        assert!(body.contains("NATIVE (6)"), "missing NATIVE header");
        assert!(
            body.contains("OPENAI-COMPAT (31)"),
            "missing OPENAI-COMPAT header"
        );
    }

    #[test]
    fn execute_lists_known_native_providers() {
        let body = render();
        // Spot-check the 6 native providers (GHOST-003: 4 stubs removed; v0.1.40: openai-codex added).
        for id in [
            "openai",
            "anthropic",
            "gemini",
            "xai",
            "bedrock",
            "openai-codex",
        ] {
            assert!(
                body.contains(id),
                "native provider id `{}` missing from output; body:\n{}",
                id,
                body
            );
        }
    }

    #[test]
    fn execute_lists_known_compat_providers() {
        let body = render();
        // Spot-check 8 of the 31 OpenAI-compat providers.
        for id in [
            "ollama",
            "groq",
            "deepseek",
            "openrouter",
            "mistral",
            "perplexity",
            "fireworks",
            "qwen",
        ] {
            assert!(
                body.contains(id),
                "compat provider id `{}` missing from output; body:\n{}",
                id,
                body
            );
        }
    }

    #[test]
    fn execute_capabilities_lists_codex_agentic_surface_support() {
        let body = render_args(&["capabilities"]);
        assert!(body.contains("Archon provider capability matrix"));
        assert!(body.contains("| `openai-codex` |"));
        assert!(body.contains("provider-neutral pipelines"));
        assert!(body.contains("subagents, /btw"));
    }

    #[test]
    fn cli_handle_capabilities_renders_without_error() {
        handle_providers(
            Some(ProvidersAction::Capabilities),
            &archon_core::config::ArchonConfig::default(),
        )
        .expect("capabilities output");
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
        let body =
            render_provider_doctor_with_pinger(false, None, None, false, false, env, &pinger);
        assert!(body.contains("ANTHROPIC_API_KEY env: OAuth token shaped"));
        assert!(body.contains("Anthropic base URL: custom via ANTHROPIC_BASE_URL"));
        assert!(body.contains("Proxy env:       set"));
        assert!(
            body.contains("Anthropic spoof identity: active for OAuth-shaped ANTHROPIC_API_KEY")
        );
        assert!(body.contains("Codex missing: run `archon auth login --provider openai-codex`"));
        assert!(!body.contains("sk-ant-oat"));
    }

    #[test]
    fn render_provider_doctor_marks_codex_kill_switch() {
        let body = render_provider_doctor_from_json(false, None, true);
        assert!(body.contains("Codex OAuth:     disabled by ARCHON_CODEX_DISABLED"));
    }

    #[test]
    fn execute_total_row_count_matches_registry_size() {
        // Render and count the data rows (lines starting with two
        // spaces and a non-dash, non-`id` character — i.e. provider
        // rows, not the header or divider). Must equal 37 (6 native + 31 compat).
        let body = render();
        let row_count = body
            .lines()
            .filter(|l| l.starts_with("  ") && !l.starts_with("  -") && !l.starts_with("  id "))
            .count();
        assert_eq!(
            row_count, 37,
            "expected exactly 37 provider rows; got {}; body:\n{}",
            row_count, body
        );
    }

    #[test]
    fn fmt_features_renders_compact_csv_or_none() {
        let all = ProviderFeatures {
            streaming: true,
            tool_calling: true,
            vision: true,
            embeddings: true,
            json_mode: true,
        };
        assert_eq!(fmt_features(&all), "stream,tools,vision,embed,json");

        let none = ProviderFeatures {
            streaming: false,
            tool_calling: false,
            vision: false,
            embeddings: false,
            json_mode: false,
        };
        assert_eq!(fmt_features(&none), "(none)");

        let only_stream = ProviderFeatures {
            streaming: true,
            tool_calling: false,
            vision: false,
            embeddings: false,
            json_mode: false,
        };
        assert_eq!(fmt_features(&only_stream), "stream");
    }

    #[test]
    fn truncate_chars_appends_ellipsis_only_when_over() {
        // Short strings unchanged.
        assert_eq!(truncate_chars("hello", 10), "hello");
        // Exact length unchanged.
        assert_eq!(truncate_chars("hello", 5), "hello");
        // Long strings truncated with ellipsis.
        let long = "abcdefghijklmnop"; // 16 chars
        let truncated = truncate_chars(long, 10);
        assert_eq!(truncated.chars().count(), 10);
        assert!(truncated.ends_with('…'));
        // Multi-byte safe.
        assert_eq!(
            truncate_chars("αβγδεζηθικ", 5).chars().count(),
            5,
            "char-count must respect codepoints, not bytes"
        );
    }

    #[test]
    fn execute_does_not_list_stripped_providers() {
        // GHOST-003 stripped the 4 stub native providers (azure, cohere,
        // copilot, minimax) entirely from NATIVE_REGISTRY. They must NOT
        // appear in /providers output at all (no [gap] marker, no row).
        let body = render();
        for id in ["azure", "cohere", "copilot", "minimax"] {
            assert!(
                !body.contains(id),
                "stripped stub provider `{}` must not appear in /providers \
                 output; body:\n{}",
                id,
                body
            );
        }
    }

    #[test]
    fn description_and_aliases() {
        let h = ProvidersHandler;
        assert!(!h.description().is_empty());
        assert_eq!(h.aliases(), &[] as &[&'static str]);
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn providers_dispatches_via_registry() {
        // Gate 5 smoke: Registry::get("providers") must return Some,
        // and execute must emit a single TextDelta with both section
        // headers + the 40-total marker.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("providers")
            .expect("providers must be registered in default_registry()");

        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        let body = match &events[0] {
            TuiEvent::TextDelta(s) => s.clone(),
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("37 total"));
        assert!(body.contains("NATIVE (6)"));
        assert!(body.contains("OPENAI-COMPAT (31)"));
    }
}
