use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::auth::{AuthError, CodexCredentials, parse_codex_credentials_json};
use crate::oauth_codex::CodexOAuthClient;

pub fn read_codex_credentials_locked(
    path: &Path,
) -> Result<(CodexCredentials, SystemTime), AuthError> {
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
            .map_err(AuthError::IoError)?;
        buf
    };

    let mtime = fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let creds = parse_codex_credentials_json(&content)?;
    Ok((creds, mtime))
}

pub fn write_codex_credentials_atomic(
    path: &Path,
    creds: &CodexCredentials,
) -> Result<(), AuthError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut root = read_existing_json(path)?;
    root["openaiCodexOauth"] = serde_json::json!({
        "accessToken": creds.access_token.expose(),
        "refreshToken": creds.refresh_token.expose(),
        "expiresAt": creds.expires_at.timestamp_millis(),
        "accountId": creds.account_id,
    });

    let content = serde_json::to_string_pretty(&root)
        .map_err(|e| AuthError::ParseError(format!("failed to serialize credentials: {e}")))?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
    }

    fs::rename(&tmp_path, path)?;
    Ok(())
}

fn read_existing_json(path: &Path) -> Result<serde_json::Value, AuthError> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content)
        .map_err(|e| AuthError::ParseError(format!("invalid existing credentials JSON: {e}")))
}

pub async fn ensure_codex_token_valid(
    path: &Path,
    client: &CodexOAuthClient,
) -> Result<CodexCredentials, AuthError> {
    let (creds, _mtime) = read_codex_credentials_locked(path)?;
    if !creds.is_expired() {
        return Ok(creds);
    }

    let refreshed = client.refresh(creds.refresh_token.expose()).await?;
    write_codex_credentials_atomic(path, &refreshed)?;
    Ok(refreshed)
}
