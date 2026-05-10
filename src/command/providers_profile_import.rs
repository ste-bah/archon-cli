//! Import local provider credentials into the Cozo auth profile store.

use anyhow::Result;
use archon_learning::provider_auth_profiles::{
    ProviderAuthProfileRecord, get_provider_auth_profile, insert_provider_auth_profile,
};
use archon_llm::auth::{CodexCredentials, OAuthCredentials};
use archon_llm::providers::descriptor::AuthFlavor;
use chrono::Utc;
use cozo::DbInstance;
use sha2::{Digest, Sha256};

pub(crate) fn import_provider_profiles() -> Result<String> {
    let db = open_learning_db()?;
    archon_learning::schema::ensure_learning_schema(&db)?;

    let profiles = discover_profiles();
    let mut imported = Vec::new();
    for profile in profiles {
        imported.push(upsert_profile(&db, profile)?);
    }
    Ok(render_imported_profiles(&imported))
}

fn discover_profiles() -> Vec<ProviderAuthProfileRecord> {
    let now = Utc::now().to_rfc3339();
    let mut profiles = Vec::new();
    profiles.extend(discover_anthropic_profiles(&now));
    profiles.extend(discover_codex_profiles(&now));
    profiles.extend(discover_native_env_profiles(&now));
    profiles.extend(discover_compat_env_profiles(&now));
    profiles
}

fn discover_anthropic_profiles(now: &str) -> Vec<ProviderAuthProfileRecord> {
    let mut profiles = Vec::new();
    let path = archon_llm::tokens::credentials_path();
    if let Ok((creds, _mtime)) = archon_llm::tokens::read_credentials_locked(&path) {
        profiles.push(anthropic_oauth_profile(&creds, now));
    }
    if let Ok(value) = std::env::var("ANTHROPIC_API_KEY")
        && !value.trim().is_empty()
    {
        profiles.push(api_key_profile(
            "anthropic-api-key-env",
            "anthropic",
            "Anthropic API key env",
            "ANTHROPIC_API_KEY",
            &value,
            now,
        ));
    }
    profiles
}

fn discover_codex_profiles(now: &str) -> Vec<ProviderAuthProfileRecord> {
    let mut profiles = Vec::new();
    let path = archon_llm::tokens::credentials_path();
    if let Ok(raw) = std::fs::read_to_string(&path)
        && let Ok(creds) = archon_llm::auth::parse_codex_credentials_json(&raw)
    {
        profiles.push(codex_oauth_profile(&creds, "archon_store", now));
    }

    let codex_cli_path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".codex")
        .join("auth.json");
    if let Ok(raw) = std::fs::read_to_string(codex_cli_path)
        && let Ok(creds) = archon_llm::auth::parse_codex_cli_credentials_json(&raw)
    {
        profiles.push(codex_oauth_profile(&creds, "external_codex_cli", now));
    }
    profiles
}

fn discover_native_env_profiles(now: &str) -> Vec<ProviderAuthProfileRecord> {
    let mut profiles = Vec::new();
    if let Ok(value) = std::env::var("OPENAI_API_KEY")
        && !value.trim().is_empty()
    {
        profiles.push(api_key_profile(
            "openai-api-key-env",
            "openai",
            "OpenAI API key env",
            "OPENAI_API_KEY",
            &value,
            now,
        ));
    }
    profiles
}

fn discover_compat_env_profiles(now: &str) -> Vec<ProviderAuthProfileRecord> {
    let mut profiles = Vec::new();
    for descriptor in archon_llm::providers::list_compat() {
        if matches!(descriptor.auth_flavor, AuthFlavor::None) || descriptor.env_key_var.is_empty() {
            continue;
        }
        let Ok(value) = std::env::var(&descriptor.env_key_var) else {
            continue;
        };
        if value.trim().is_empty() {
            continue;
        }
        profiles.push(api_key_profile(
            format!("{}-api-key-env", descriptor.id),
            descriptor.id.clone(),
            descriptor.display_name.clone(),
            descriptor.env_key_var.clone(),
            &value,
            now,
        ));
    }
    profiles
}

fn anthropic_oauth_profile(creds: &OAuthCredentials, now: &str) -> ProviderAuthProfileRecord {
    ProviderAuthProfileRecord::new(
        "anthropic-oauth-archon",
        "anthropic",
        "oauth",
        "archon_store",
        now,
    )
    .with_display_name("Anthropic OAuth")
    .with_identity_fingerprint(secret_fingerprint(creds.refresh_token.expose()))
    .with_redacted_metadata(serde_json::json!({
        "spoof_capable": true,
        "expires_at": creds.expires_at.to_rfc3339(),
        "expired": creds.is_expired(),
        "subscription_type": creds.subscription_type,
        "scope_count": creds.scopes.len(),
    }))
}

fn codex_oauth_profile(
    creds: &CodexCredentials,
    source: &str,
    now: &str,
) -> ProviderAuthProfileRecord {
    ProviderAuthProfileRecord::new(
        format!("openai-codex-oauth-{source}"),
        "openai-codex",
        "oauth",
        source,
        now,
    )
    .with_display_name("OpenAI Codex OAuth")
    .with_identity_fingerprint(secret_fingerprint(creds.refresh_token.expose()))
    .with_redacted_metadata(serde_json::json!({
        "account_id_sha256": value_hash(&creds.account_id),
        "expires_at": creds.expires_at.to_rfc3339(),
        "expired": creds.is_expired(),
        "provider_injection": false,
    }))
}

fn api_key_profile(
    profile_id: impl Into<String>,
    provider_id: impl Into<String>,
    display_name: impl Into<String>,
    env_var: impl Into<String>,
    secret_value: &str,
    now: &str,
) -> ProviderAuthProfileRecord {
    let env_var = env_var.into();
    ProviderAuthProfileRecord::new(profile_id, provider_id, "api_key", "env", now)
        .with_display_name(display_name)
        .with_identity_fingerprint(secret_fingerprint(secret_value))
        .with_redacted_metadata(serde_json::json!({
            "env_var": env_var,
            "secret_present": true,
        }))
}

fn upsert_profile(
    db: &DbInstance,
    profile: ProviderAuthProfileRecord,
) -> Result<ProviderAuthProfileRecord> {
    let merged = match get_provider_auth_profile(db, &profile.profile_id)? {
        Some(existing) => merge_profile(existing, profile),
        None => profile,
    };
    insert_provider_auth_profile(db, &merged)?;
    Ok(merged)
}

fn merge_profile(
    mut existing: ProviderAuthProfileRecord,
    discovered: ProviderAuthProfileRecord,
) -> ProviderAuthProfileRecord {
    existing.provider_id = discovered.provider_id;
    existing.auth_kind = discovered.auth_kind;
    existing.display_name = discovered.display_name;
    existing.source = discovered.source;
    existing.identity_fingerprint = discovered.identity_fingerprint;
    existing.updated_at = discovered.updated_at;
    existing.metadata_redacted_json = discovered.metadata_redacted_json;
    existing
}

fn render_imported_profiles(profiles: &[ProviderAuthProfileRecord]) -> String {
    if profiles.is_empty() {
        return "No local provider auth profiles found to import.\n".into();
    }

    let mut out = String::from("Imported provider auth profiles (Cozo)\n\n");
    out.push_str(&format!(
        "{:<34} {:<16} {:<10} {:<18} status\n",
        "profile_id", "provider", "auth", "source"
    ));
    for profile in profiles {
        out.push_str(&format!(
            "{:<34} {:<16} {:<10} {:<18} {}\n",
            profile.profile_id,
            profile.provider_id,
            profile.auth_kind,
            profile.source,
            profile_status(profile)
        ));
    }
    out.push_str(&format!("\n{} profile(s) imported.\n", profiles.len()));
    out
}

fn profile_status(profile: &ProviderAuthProfileRecord) -> &'static str {
    if profile.cooldown_until.is_some() {
        "cooldown-preserved"
    } else if profile.disabled_reason.is_some() {
        "disabled"
    } else {
        "ok"
    }
}

fn open_learning_db() -> Result<DbInstance> {
    let base = archon_session::storage::default_db_path();
    let parent = base
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory"))?;
    let path = parent.join("learning.db");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path_str = path.to_string_lossy().to_string();
    DbInstance::new("sqlite", &path_str, "").map_err(|e| anyhow::anyhow!("open learning db: {e}"))
}

fn secret_fingerprint(value: &str) -> String {
    format!("sha256:{}", value_hash(value))
}

fn value_hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!(
            "/tmp/test-provider-profile-import-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn api_key_profile_hashes_secret_without_storing_value() {
        let profile = api_key_profile(
            "openai-api-key-env",
            "openai",
            "OpenAI API key env",
            "OPENAI_API_KEY",
            "sk-secret-value",
            "2026-05-08T12:00:00Z",
        );
        let raw = serde_json::to_string(&profile).unwrap();

        assert_eq!(profile.identity_fingerprint.as_deref().unwrap().len(), 71);
        assert_eq!(profile.metadata_redacted_json["env_var"], "OPENAI_API_KEY");
        assert!(!raw.contains("sk-secret-value"));
    }

    #[test]
    fn upsert_preserves_cooldown_and_failure_state() {
        let db = test_db();
        let existing = ProviderAuthProfileRecord::new(
            "anthropic-api-key-env",
            "anthropic",
            "api_key",
            "env",
            "2026-05-08T11:00:00Z",
        )
        .with_usage(None, None, Some("2026-05-08T11:10:00Z".into()), 3)
        .with_cooldown("2026-05-08T13:00:00Z", "rate_limited");
        insert_provider_auth_profile(&db, &existing).unwrap();

        let discovered = api_key_profile(
            "anthropic-api-key-env",
            "anthropic",
            "Anthropic API key env",
            "ANTHROPIC_API_KEY",
            "sk-new",
            "2026-05-08T12:00:00Z",
        );
        let imported = upsert_profile(&db, discovered).unwrap();

        assert_eq!(imported.failure_count, 3);
        assert_eq!(
            imported.cooldown_until.as_deref(),
            Some("2026-05-08T13:00:00Z")
        );
        assert_eq!(imported.updated_at, "2026-05-08T12:00:00Z");
    }

    #[test]
    fn render_import_mentions_preserved_cooldown() {
        let profile = ProviderAuthProfileRecord::new(
            "p1",
            "anthropic",
            "oauth",
            "archon_store",
            "2026-05-08T12:00:00Z",
        )
        .with_cooldown("2026-05-08T13:00:00Z", "usage_limited");

        let rendered = render_imported_profiles(&[profile]);

        assert!(rendered.contains("cooldown-preserved"));
    }
}
