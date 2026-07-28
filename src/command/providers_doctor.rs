//! Provider doctor diagnostics for `providers`.

use chrono::Utc;

#[cfg(test)]
use crate::command::providers_live::DisabledLivePinger;
use crate::command::providers_live::{ProviderLivePinger, TcpProviderLivePinger};

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

#[cfg(test)]
#[path = "providers_doctor_tests.rs"]
mod tests;
