use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;

use crate::cli_args::{AuthArgs, AuthProviderKind, AuthSubcommand};

pub async fn handle_auth(args: AuthArgs, config: &archon_core::config::ArchonConfig) -> Result<()> {
    match args.command {
        AuthSubcommand::Login {
            provider,
            accept_tos,
        } => login(provider, accept_tos, config).await,
        AuthSubcommand::Status => status(config).await,
        AuthSubcommand::Logout { provider } => logout(provider),
    }
}

async fn login(
    provider: AuthProviderKind,
    accept_tos: bool,
    config: &archon_core::config::ArchonConfig,
) -> Result<()> {
    match provider {
        AuthProviderKind::Anthropic => crate::command::login::handle_login(config).await,
        AuthProviderKind::OpenaiCodex => {
            if !accept_tos && !tos_ack_path().exists() && !prompt_tos()? {
                eprintln!("Codex login cancelled.");
                return Ok(());
            }
            let http = reqwest::Client::new();
            let client = archon_llm::oauth_codex::CodexOAuthClient::new(http);
            eprintln!("Starting Codex OAuth login...");
            let creds = client
                .login(|url| {
                    eprintln!("Open this URL to continue Codex login: {url}");
                })
                .await
                .context("Codex OAuth login failed")?;
            archon_llm::tokens_codex::write_codex_credentials_atomic(
                &archon_llm::tokens::credentials_path(),
                &creds,
            )
            .context("failed to store Codex credentials")?;
            if !accept_tos {
                write_tos_ack()?;
            }
            eprintln!("Codex login successful.");
            Ok(())
        }
    }
}

async fn status(config: &archon_core::config::ArchonConfig) -> Result<()> {
    let path = archon_llm::tokens::credentials_path();
    println!("Anthropic (Claude)");
    match read_file(&path).and_then(|json| archon_llm::auth::parse_credentials_json(&json).ok()) {
        Some(creds) => {
            println!("  Status:        authenticated");
            println!("  Token expires: {}", format_time(creds.expires_at));
            println!("  Subscription:  {}", creds.subscription_type);
        }
        None => println!(
            "  Status:        not authenticated. Run: archon auth login --provider anthropic"
        ),
    }

    println!("\nCodex (OpenAI ChatGPT subscription)");
    if codex_disabled() {
        println!("  Status:           DISABLED via ARCHON_CODEX_DISABLED=1");
        return Ok(());
    }

    match read_file(&path)
        .and_then(|json| archon_llm::auth::parse_codex_credentials_json(&json).ok())
    {
        Some(creds) => {
            println!(
                "  Status:           authenticated as account {}",
                redact_account(&creds.account_id)
            );
            println!("  Token expires:    {}", format_time(creds.expires_at));
            print_spoof_status(config).await?;
            println!("  Kill-switch:      enabled (set ARCHON_CODEX_DISABLED=1 to disable)");
        }
        None => {
            println!(
                "  Status:           not authenticated. Run: archon auth login --provider openai-codex"
            );
        }
    }
    Ok(())
}

async fn print_spoof_status(config: &archon_core::config::ArchonConfig) -> Result<()> {
    let codex_cfg = codex_config_from_core(&config.providers.openai_codex);
    match archon_llm::providers::codex::spoof::resolve(&codex_cfg, &reqwest::Client::new()).await {
        Ok(resolution) => {
            println!(
                "  Spoof identity:   from {}",
                source_label(&resolution.primary_source)
            );
            println!("    originator:     {}", resolution.config.originator);
            println!("    user-agent:     {}", resolution.config.user_agent);
            println!(
                "    client-id:      {}",
                redact_client_id(&resolution.config.client_id)
            );
            println!("    openai-beta:    {}", resolution.config.openai_beta);
        }
        Err(err) => println!("  Spoof identity:   unavailable ({err})"),
    }
    println!(
        "  Manifest:         {}",
        config.providers.openai_codex.manifest.fetch_url
    );
    Ok(())
}

fn logout(provider: Option<AuthProviderKind>) -> Result<()> {
    let path = archon_llm::tokens::credentials_path();
    logout_path(&path, provider)
}

fn logout_path(path: &Path, provider: Option<AuthProviderKind>) -> Result<()> {
    if !path.exists() {
        println!("No stored credentials found.");
        return Ok(());
    }
    let mut root: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path)?).unwrap_or_else(|_| serde_json::json!({}));
    match provider {
        Some(AuthProviderKind::Anthropic) => {
            remove_key(&mut root, "claudeAiOauth");
        }
        Some(AuthProviderKind::OpenaiCodex) => {
            remove_key(&mut root, "openaiCodexOauth");
        }
        None => {
            remove_key(&mut root, "claudeAiOauth");
            remove_key(&mut root, "openaiCodexOauth");
        }
    }

    if root.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        fs::remove_file(path)?;
    } else {
        write_json_atomic(path, &root)?;
    }
    println!("Credentials updated.");
    Ok(())
}

pub(crate) fn codex_config_from_core(
    cfg: &archon_core::config::CodexProviderConfig,
) -> archon_llm::providers::codex::spoof::CodexProviderConfig {
    archon_llm::providers::codex::spoof::CodexProviderConfig {
        enabled: cfg.enabled,
        spoof: archon_llm::providers::codex::spoof::CodexSpoofPartialConfig {
            originator: cfg.spoof.originator.clone(),
            user_agent: cfg.spoof.user_agent.clone(),
            client_id: cfg.spoof.client_id.clone(),
            openai_beta: cfg.spoof.openai_beta.clone(),
            extra_headers: cfg.spoof.extra_headers.clone(),
        },
        manifest: archon_llm::providers::codex::spoof::CodexManifestConfig {
            fetch_url: cfg.manifest.fetch_url.clone(),
            ttl_seconds: cfg.manifest.ttl_seconds,
            cache_dir: cfg.manifest.cache_dir.clone(),
        },
    }
}

fn prompt_tos() -> Result<bool> {
    eprintln!("{}", TOS_WARNING);
    eprint!("Continue with Codex login? [y/N] ");
    io::stderr().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let yes = input.trim().eq_ignore_ascii_case("y");
    if yes {
        write_tos_ack()?;
    }
    Ok(yes)
}

fn tos_ack_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".archon")
        .join("codex-tos-ack")
}

fn write_tos_ack() -> Result<()> {
    let path = tos_ack_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        path,
        format!("acknowledged_at: {}\n", Utc::now().to_rfc3339()),
    )?;
    Ok(())
}

fn read_file(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn remove_key(root: &mut serde_json::Value, key: &str) {
    if let Some(obj) = root.as_object_mut() {
        obj.remove(key);
    }
}

fn write_json_atomic(path: &Path, value: &serde_json::Value) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn codex_disabled() -> bool {
    std::env::var("ARCHON_CODEX_DISABLED")
        .map(|v| codex_disabled_value(&v))
        .unwrap_or(false)
}

fn codex_disabled_value(value: &str) -> bool {
    matches!(value.to_lowercase().as_str(), "1" | "true" | "yes")
}

fn format_time(dt: chrono::DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M UTC").to_string()
}

fn redact_account(id: &str) -> String {
    let suffix: String = id
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("***...{suffix}")
}

fn redact_client_id(id: &str) -> String {
    let prefix: String = id.chars().take(14).collect();
    format!("{prefix}...")
}

fn source_label(source: &archon_llm::providers::codex::spoof::ResolvedSource) -> &'static str {
    match source {
        archon_llm::providers::codex::spoof::ResolvedSource::EnvVar => "environment variables",
        archon_llm::providers::codex::spoof::ResolvedSource::ConfigToml => "config.toml",
        archon_llm::providers::codex::spoof::ResolvedSource::FetchedManifest { .. } => {
            "fetched manifest"
        }
        archon_llm::providers::codex::spoof::ResolvedSource::BundledManifest => "bundled manifest",
    }
}

const TOS_WARNING: &str = r#"WARNING: Codex authentication via archon-cli

archon-cli authenticates against ChatGPT subscription tokens (auth.openai.com)
and uses an undocumented internal API at chatgpt.com/backend-api/codex.

Risks:
  1. OpenAI may change or restrict this API without notice.
  2. ChatGPT subscription terms may restrict programmatic access.
  3. By default, archon-cli identifies as 'openclaw'. You can override this in config.toml.

Legal guardrail:
  - archon-cli REJECTS user-agent strings starting with 'ChatGPT/', 'OpenAI/',
    'ChatGPT-', or 'OpenAI-' to prevent impersonation of OpenAI's own products.
  - You are SOLELY responsible for any other identity choice you configure.

Mitigations:
  - Disable Codex entirely: set ARCHON_CODEX_DISABLED=1
  - Customize identity: edit [providers.openai-codex.spoof] in config.toml
  - Hot-update spoof config: manifest refreshes every 6 hours by default
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn credential_file() -> serde_json::Value {
        serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "anthropic-access",
                "refreshToken": "anthropic-refresh",
                "expiresAt": 4070908800000i64,
                "scopes": ["user:inference"],
                "subscriptionType": "pro"
            },
            "openaiCodexOauth": {
                "accessToken": "codex-access",
                "refreshToken": "codex-refresh",
                "expiresAt": 4070908800000i64,
                "accountId": "acct_1234567890"
            }
        })
    }

    #[test]
    fn codex_disabled_value_accepts_documented_truthy_values() {
        for value in ["1", "true", "TRUE", "yes", "YES"] {
            assert!(codex_disabled_value(value), "{value} should disable Codex");
        }
        for value in ["", "0", "false", "no", "enabled"] {
            assert!(
                !codex_disabled_value(value),
                "{value} should not disable Codex"
            );
        }
    }

    #[test]
    fn redact_account_keeps_only_last_four_chars() {
        assert_eq!(redact_account("acct_1234567890"), "***...7890");
    }

    #[test]
    fn redact_client_id_keeps_only_diagnostic_prefix() {
        assert_eq!(
            redact_client_id("app_EMoamEEZ73f0CkXaXp7hrann"),
            "app_EMoamEEZ73..."
        );
    }

    #[test]
    fn logout_path_removes_only_codex_credentials() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join(".credentials.json");
        fs::write(&path, serde_json::to_string_pretty(&credential_file())?)?;

        logout_path(&path, Some(AuthProviderKind::OpenaiCodex))?;

        let saved: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
        assert!(saved.get("claudeAiOauth").is_some());
        assert!(saved.get("openaiCodexOauth").is_none());
        Ok(())
    }

    #[test]
    fn logout_path_without_provider_removes_empty_file() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join(".credentials.json");
        fs::write(&path, serde_json::to_string_pretty(&credential_file())?)?;

        logout_path(&path, None)?;

        assert!(!path.exists());
        Ok(())
    }
}
