use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::auth::{AuthError, OAuthCredentials, parse_credentials_json};
use crate::types::Secret;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const TOKEN_ENDPOINT: &str = "https://platform.claude.com/v1/oauth/token";
const MAX_RETRIES: u32 = 5;
const LOCK_RETRY_MIN_MS: u64 = 1000;
const LOCK_RETRY_MAX_MS: u64 = 2000;

// ---------------------------------------------------------------------------
// Token refresh
// ---------------------------------------------------------------------------

/// Refresh OAuth tokens by POSTing to the Anthropic token endpoint.
///
/// Returns updated `OAuthCredentials` or an error.
pub async fn refresh_token(
    refresh_token: &Secret<String>,
    client: &reqwest::Client,
) -> Result<OAuthCredentials, AuthError> {
    let response = client
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.expose()),
        ])
        .send()
        .await
        .map_err(|e| AuthError::ParseError(format!("token refresh HTTP error: {e}")))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| AuthError::ParseError(format!("failed to read refresh response: {e}")))?;

    if !status.is_success() {
        if body.contains("invalid_grant") || body.contains("expired") {
            return Err(AuthError::NoCredentials(
                "Refresh token expired. Run `archon login` to re-authenticate.".into(),
            ));
        }
        // Truncate body to avoid leaking tokens in error messages
        let safe_body: String = body.chars().take(200).collect();
        return Err(AuthError::ParseError(format!(
            "token refresh failed (HTTP {status}): {safe_body}"
        )));
    }

    // Parse the response as a credential file format
    // The token endpoint returns the same shape as claudeAiOauth
    parse_refresh_response(&body)
}

/// Parse the token endpoint response into `OAuthCredentials`.
fn parse_refresh_response(body: &str) -> Result<OAuthCredentials, AuthError> {
    #[derive(serde::Deserialize)]
    struct TokenResponse {
        access_token: String,
        refresh_token: String,
        expires_in: Option<u64>,
        #[serde(default)]
        scope: String,
    }

    let resp: TokenResponse = serde_json::from_str(body)
        .map_err(|e| AuthError::ParseError(format!("invalid token response: {e}")))?;

    let expires_at =
        chrono::Utc::now() + chrono::Duration::seconds(resp.expires_in.unwrap_or(3600) as i64);

    let scopes: Vec<String> = resp
        .scope
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    Ok(OAuthCredentials {
        access_token: Secret::new(resp.access_token),
        refresh_token: Secret::new(resp.refresh_token),
        expires_at,
        scopes,
        subscription_type: String::new(), // Not returned by refresh endpoint
    })
}

// ---------------------------------------------------------------------------
// Credential file operations with file locking
// ---------------------------------------------------------------------------

/// Default credential file path.
pub fn credentials_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join(".credentials.json")
}

/// Read the credential file with an advisory file lock.
///
/// Uses `fd-lock` for cross-process locking. Retries up to MAX_RETRIES
/// times with random delay if the lock is held.
pub fn read_credentials_locked(path: &Path) -> Result<(OAuthCredentials, SystemTime), AuthError> {
    let file = fs::OpenOptions::new().read(true).open(path)?;

    let lock = fd_lock::RwLock::new(file);
    let guard = lock
        .try_read()
        .map_err(|e| AuthError::ParseError(format!("failed to acquire read lock: {e}")))?;

    let content = {
        use std::io::Read;
        let mut buf = String::new();
        (&*guard)
            .read_to_string(&mut buf)
            .map_err(|e| AuthError::IoError(e))?;
        buf
    };

    let mtime = fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let creds = parse_credentials_json(&content)?;
    Ok((creds, mtime))
}

/// Write credentials to the file atomically (write to .tmp, then rename).
///
/// Sets file permissions to 0600.
pub fn write_credentials_atomic(path: &Path, creds: &OAuthCredentials) -> Result<(), AuthError> {
    let tmp_path = path.with_extension("json.tmp");

    // Build the credential JSON in Claude Code's format
    let json = serde_json::json!({
        "claudeAiOauth": {
            "accessToken": creds.access_token.expose(),
            "refreshToken": creds.refresh_token.expose(),
            "expiresAt": creds.expires_at.timestamp_millis(),
            "scopes": creds.scopes,
            "subscriptionType": creds.subscription_type,
        }
    });

    let content = serde_json::to_string_pretty(&json)
        .map_err(|e| AuthError::ParseError(format!("failed to serialize credentials: {e}")))?;

    // Write to temp file
    fs::write(&tmp_path, &content)?;

    // Set permissions on temp file
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
    }

    // Atomic rename
    fs::rename(&tmp_path, path)?;

    Ok(())
}

/// Refresh credentials if expired, with file locking and mtime check.
///
/// 1. Read credential file with lock, note mtime
/// 2. If token not expired, return as-is
/// 3. Attempt to acquire write lock (retry with backoff)
/// 4. After acquiring lock, re-check mtime -- if changed, another process refreshed
/// 5. If still expired, perform HTTP refresh
/// 6. Write atomically
pub async fn refresh_if_needed(
    path: &Path,
    client: &reqwest::Client,
) -> Result<OAuthCredentials, AuthError> {
    let (creds, initial_mtime) = read_credentials_locked(path)?;

    if !creds.is_expired() {
        return Ok(creds);
    }

    tracing::info!("OAuth token expired, refreshing...");

    // Retry loop for acquiring write lock
    for attempt in 0..MAX_RETRIES {
        let file = fs::OpenOptions::new().read(true).write(true).open(path)?;

        let mut lock = fd_lock::RwLock::new(file);

        match lock.try_write() {
            Ok(_guard) => {
                // Check if another process already refreshed
                let current_mtime = fs::metadata(path)
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);

                if current_mtime != initial_mtime {
                    // Re-read -- another process may have refreshed
                    tracing::debug!("credential file changed, re-reading");
                    let content = fs::read_to_string(path)?;
                    let new_creds = parse_credentials_json(&content)?;
                    if !new_creds.is_expired() {
                        return Ok(new_creds);
                    }
                }

                // Perform the actual refresh
                let new_creds = refresh_token(&creds.refresh_token, client).await?;
                write_credentials_atomic(path, &new_creds)?;
                tracing::info!("OAuth token refreshed successfully");
                return Ok(new_creds);
            }
            Err(_) => {
                if attempt < MAX_RETRIES - 1 {
                    let delay = rand_delay();
                    tracing::debug!(
                        "credential file locked, retry {}/{} in {delay:?}",
                        attempt + 1,
                        MAX_RETRIES
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    Err(AuthError::ParseError(format!(
        "failed to acquire write lock on credential file after {MAX_RETRIES} attempts"
    )))
}

fn rand_delay() -> Duration {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut hasher);
    let hash = hasher.finish();
    let ms = LOCK_RETRY_MIN_MS + (hash % (LOCK_RETRY_MAX_MS - LOCK_RETRY_MIN_MS));
    Duration::from_millis(ms)
}
