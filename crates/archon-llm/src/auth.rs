use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::types::Secret;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("credential file I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("credential parse error: {0}")]
    ParseError(String),

    #[error("no credentials found: {0}")]
    NoCredentials(String),
}

// ---------------------------------------------------------------------------
// Credential types
// ---------------------------------------------------------------------------

/// Parsed OAuth credentials from `~/.claude/.credentials.json`.
#[derive(Debug, Clone)]
pub struct OAuthCredentials {
    pub access_token: Secret<String>,
    pub refresh_token: Secret<String>,
    pub expires_at: DateTime<Utc>,
    pub scopes: Vec<String>,
    pub subscription_type: String,
}

impl OAuthCredentials {
    /// Returns true if the token is expired or within the 5-minute refresh buffer.
    pub fn is_expired(&self) -> bool {
        let buffer = chrono::Duration::minutes(5);
        Utc::now() + buffer >= self.expires_at
    }
}

/// The resolved authentication method.
#[derive(Debug, Clone)]
pub enum AuthProvider {
    ApiKey(Secret<String>),
    OAuthToken(OAuthCredentials),
    BearerToken(Secret<String>),
}

impl AuthProvider {
    /// Returns the HTTP header `(name, value)` for this auth method.
    pub fn header(&self) -> (String, String) {
        match self {
            AuthProvider::ApiKey(key) => ("x-api-key".to_string(), key.expose().clone()),
            AuthProvider::OAuthToken(creds) => (
                "Authorization".to_string(),
                format!("Bearer {}", creds.access_token.expose()),
            ),
            AuthProvider::BearerToken(token) => (
                "Authorization".to_string(),
                format!("Bearer {}", token.expose()),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// JSON deserialization helpers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CredentialFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<RawOAuthCredentials>,
}

#[derive(Deserialize)]
struct RawOAuthCredentials {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "refreshToken")]
    refresh_token: String,
    #[serde(rename = "expiresAt")]
    #[serde(deserialize_with = "deserialize_expires_at")]
    expires_at: DateTime<Utc>,
    scopes: Vec<String>,
    #[serde(rename = "subscriptionType")]
    subscription_type: String,
}

/// Deserialize `expiresAt` from either epoch-ms integer or RFC 3339 string.
/// Real Claude Code credentials use epoch-ms integers.
fn deserialize_expires_at<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct ExpiresAtVisitor;

    impl<'de> de::Visitor<'de> for ExpiresAtVisitor {
        type Value = DateTime<Utc>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("epoch-ms integer or RFC 3339 string")
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            let secs = v / 1000;
            let nsecs = ((v % 1000) * 1_000_000) as u32;
            DateTime::from_timestamp(secs, nsecs)
                .ok_or_else(|| E::custom(format!("invalid epoch-ms timestamp: {v}")))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            self.visit_i64(v as i64)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            DateTime::parse_from_rfc3339(v)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| E::custom(format!("invalid RFC 3339 date: {e}")))
        }
    }

    deserializer.deserialize_any(ExpiresAtVisitor)
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Parse a credential JSON string into `OAuthCredentials`.
pub fn parse_credentials_json(json: &str) -> Result<OAuthCredentials, AuthError> {
    let file: CredentialFile = serde_json::from_str(json)
        .map_err(|e| AuthError::ParseError(format!("invalid JSON: {e}")))?;

    let raw = file
        .claude_ai_oauth
        .ok_or_else(|| AuthError::ParseError("missing claudeAiOauth key".into()))?;

    Ok(OAuthCredentials {
        access_token: Secret::new(raw.access_token),
        refresh_token: Secret::new(raw.refresh_token),
        expires_at: raw.expires_at,
        scopes: raw.scopes,
        subscription_type: raw.subscription_type,
    })
}

/// Default credential file path: `~/.claude/.credentials.json`
pub fn default_credentials_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join(".credentials.json")
}

/// Load OAuth credentials from a file path.
pub fn load_credentials_file(path: &std::path::Path) -> Result<OAuthCredentials, AuthError> {
    // Check file permissions (advisory)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let mode = meta.permissions().mode() & 0o777;
            if mode != 0o600 {
                tracing::warn!(
                    "credential file {:?} has permissions {:o}, expected 0600",
                    path,
                    mode
                );
            }
        }
    }

    let content = fs::read_to_string(path)?;
    parse_credentials_json(&content)
}

/// Resolve authentication from available sources.
///
/// Priority: API key > Bearer token > OAuth credential file.
///
/// - `api_key`: value of `ANTHROPIC_API_KEY` env var (or None)
/// - `auth_token`: value of `ANTHROPIC_AUTH_TOKEN` env var (or None)
/// - `credentials_json`: raw JSON content of the credential file (or None)
pub fn resolve_auth(
    api_key: Option<&str>,
    auth_token: Option<&str>,
    credentials_json: Option<&str>,
) -> Result<AuthProvider, AuthError> {
    // Priority 1: API key
    if let Some(key) = api_key {
        return Ok(AuthProvider::ApiKey(Secret::new(key.to_string())));
    }

    // Priority 2: Bearer token
    if let Some(token) = auth_token {
        return Ok(AuthProvider::BearerToken(Secret::new(token.to_string())));
    }

    // Priority 3: OAuth credential file
    if let Some(json) = credentials_json {
        let creds = parse_credentials_json(json)?;
        return Ok(AuthProvider::OAuthToken(creds));
    }

    Err(AuthError::NoCredentials(
        "No credentials found. Run `archon login` or set ANTHROPIC_API_KEY environment variable."
            .into(),
    ))
}

/// Resolve authentication from pre-parsed values.
///
/// Priority: `api_key` > `archon_api_key` > `archon_oauth_token`
/// > `auth_token` (legacy) > OAuth credential file.
pub fn resolve_auth_with_keys(
    api_key: Option<&str>,
    archon_api_key: Option<&str>,
    archon_oauth_token: Option<&str>,
    auth_token: Option<&str>,
) -> Result<AuthProvider, AuthError> {
    // Priority 1: ANTHROPIC_API_KEY
    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        return Ok(AuthProvider::ApiKey(Secret::new(key.to_string())));
    }

    // Priority 2: ARCHON_API_KEY (alias)
    if let Some(key) = archon_api_key.filter(|k| !k.trim().is_empty()) {
        return Ok(AuthProvider::ApiKey(Secret::new(key.to_string())));
    }

    // Priority 3: ARCHON_OAUTH_TOKEN — pre-set OAuth token (skip login flow)
    if let Some(token) = archon_oauth_token.filter(|t| !t.trim().is_empty()) {
        return Ok(AuthProvider::BearerToken(Secret::new(token.to_string())));
    }

    // Priority 4: Legacy ANTHROPIC_AUTH_TOKEN
    if let Some(token) = auth_token.filter(|t| !t.trim().is_empty()) {
        return Ok(AuthProvider::BearerToken(Secret::new(token.to_string())));
    }

    // Priority 5: OAuth credential file
    let cred_path = default_credentials_path();
    if cred_path.exists() {
        let creds = load_credentials_file(&cred_path)?;
        return Ok(AuthProvider::OAuthToken(creds));
    }

    Err(AuthError::NoCredentials(
        "No credentials found. Run `archon login` or set ANTHROPIC_API_KEY environment variable."
            .into(),
    ))
}

/// Convenience: resolve auth from environment and default credential file.
///
/// Priority: `ANTHROPIC_API_KEY` > `ARCHON_API_KEY` > `ARCHON_OAUTH_TOKEN`
/// > `ANTHROPIC_AUTH_TOKEN` > OAuth credential file.
pub fn resolve_auth_from_env() -> Result<AuthProvider, AuthError> {
    let api_key = std::env::var("ANTHROPIC_API_KEY").ok();
    let archon_api_key = std::env::var("ARCHON_API_KEY").ok();
    let archon_oauth_token = std::env::var("ARCHON_OAUTH_TOKEN").ok();
    let auth_token = std::env::var("ANTHROPIC_AUTH_TOKEN").ok();

    resolve_auth_with_keys(
        api_key.as_deref(),
        archon_api_key.as_deref(),
        archon_oauth_token.as_deref(),
        auth_token.as_deref(),
    )
}
