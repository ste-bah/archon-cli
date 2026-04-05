//! Self-update mechanism for Archon binary.
//!
//! Checks GitHub Releases for newer versions, downloads, verifies SHA256, and
//! atomically replaces the running binary.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use semver::Version;
use sha2::{Digest, Sha256};

/// Current version of this binary.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default GitHub Releases API URL.
const DEFAULT_RELEASE_URL: &str = "https://api.github.com/repos/ste-bah/archon/releases/latest";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct UpdateConfig {
    /// "stable" or "beta"
    pub channel: String,
    /// Check for updates on startup
    pub auto_check: bool,
    /// Hours between automatic checks
    pub check_interval_hours: u64,
    /// Custom release URL (override for self-hosted)
    pub release_url: Option<String>,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            channel: "stable".to_string(),
            auto_check: true,
            check_interval_hours: 24,
            release_url: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("permission denied: cannot replace binary at {path}")]
    PermissionDenied { path: String },
    #[error("up to date: {0}")]
    UpToDate(String),
}

#[derive(Debug)]
pub struct ReleaseInfo {
    pub version: String,
    pub changelog: String,
    pub download_url: String,
    pub checksum_url: String,
}

// ---------------------------------------------------------------------------
// Platform detection
// ---------------------------------------------------------------------------

pub fn platform_asset_name() -> &'static str {
    if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "archon-linux-x86_64"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "aarch64") {
        "archon-linux-aarch64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        "archon-darwin-x86_64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "archon-darwin-aarch64"
    } else {
        "archon-linux-x86_64" // fallback
    }
}

// ---------------------------------------------------------------------------
// Version check
// ---------------------------------------------------------------------------

pub async fn check_latest(config: &UpdateConfig) -> Result<ReleaseInfo, UpdateError> {
    let url = config.release_url.as_deref().unwrap_or(DEFAULT_RELEASE_URL);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(format!("archon/{}", CURRENT_VERSION))
        .build()
        .map_err(|e| UpdateError::Http(e.to_string()))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| UpdateError::Http(format!("request failed: {e}")))?;

    if !response.status().is_success() {
        return Err(UpdateError::Http(format!("HTTP {}", response.status())));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| UpdateError::Parse(e.to_string()))?;

    let version = json["tag_name"]
        .as_str()
        .ok_or_else(|| UpdateError::Parse("missing tag_name".into()))?
        .trim_start_matches('v')
        .to_string();

    let changelog = json["body"].as_str().unwrap_or("").to_string();

    let asset_name = platform_asset_name();
    let assets = json["assets"]
        .as_array()
        .ok_or_else(|| UpdateError::Parse("missing assets".into()))?;

    let download_url = assets
        .iter()
        .find(|a| a["name"].as_str() == Some(asset_name))
        .and_then(|a| a["browser_download_url"].as_str())
        .ok_or_else(|| UpdateError::Parse(format!("asset '{asset_name}' not found in release")))?
        .to_string();

    let checksum_url = assets
        .iter()
        .find(|a| a["name"].as_str() == Some("checksums.sha256"))
        .and_then(|a| a["browser_download_url"].as_str())
        .unwrap_or("")
        .to_string();

    Ok(ReleaseInfo {
        version,
        changelog,
        download_url,
        checksum_url,
    })
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (Version::parse(latest), Version::parse(current)) {
        (Ok(l), Ok(c)) => l > c,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Download + verify
// ---------------------------------------------------------------------------

pub async fn download_and_verify(release: &ReleaseInfo) -> Result<Vec<u8>, UpdateError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .user_agent(format!("archon/{}", CURRENT_VERSION))
        .build()
        .map_err(|e| UpdateError::Http(e.to_string()))?;

    // Download binary
    let bytes = client
        .get(&release.download_url)
        .send()
        .await
        .map_err(|e| UpdateError::Http(format!("download failed: {e}")))?
        .bytes()
        .await
        .map_err(|e| UpdateError::Http(format!("read failed: {e}")))?;

    // Verify checksum if available
    if !release.checksum_url.is_empty() {
        let checksum_text = client
            .get(&release.checksum_url)
            .send()
            .await
            .map_err(|e| UpdateError::Http(format!("checksum download failed: {e}")))?
            .text()
            .await
            .map_err(|e| UpdateError::Http(format!("checksum read failed: {e}")))?;

        let asset_name = platform_asset_name();
        let expected = parse_expected_checksum(&checksum_text, asset_name)
            .ok_or_else(|| UpdateError::Parse(format!("checksum for '{asset_name}' not found")))?;

        let actual = compute_sha256(&bytes);

        if actual != expected {
            return Err(UpdateError::ChecksumMismatch { expected, actual });
        }
    }

    Ok(bytes.to_vec())
}

/// Parse SHA256 checksum for a specific filename from a checksums file.
/// Format: `<sha256hex>  <filename>`
fn parse_expected_checksum(text: &str, filename: &str) -> Option<String> {
    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(2, "  ").collect();
        if parts.len() == 2 && parts[1].trim() == filename {
            return Some(parts[0].trim().to_string());
        }
    }
    None
}

pub fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

// ---------------------------------------------------------------------------
// Binary replacement
// ---------------------------------------------------------------------------

pub fn replace_binary(new_binary: Vec<u8>) -> Result<PathBuf, UpdateError> {
    let current_exe = std::env::current_exe()?;
    let parent = current_exe
        .parent()
        .ok_or_else(|| std::io::Error::other("cannot determine binary dir"))?;

    let temp_path = parent.join(format!("archon.tmp.{}", std::process::id()));

    // Write binary to temp file
    if let Err(e) = std::fs::write(&temp_path, &new_binary) {
        return Err(UpdateError::Io(e));
    }

    // Set executable bit (Unix only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        if let Err(e) = std::fs::set_permissions(&temp_path, perms) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(UpdateError::Io(e));
        }
    }

    // Atomic rename
    if let Err(e) = std::fs::rename(&temp_path, &current_exe) {
        let _ = std::fs::remove_file(&temp_path);
        return if e.kind() == std::io::ErrorKind::PermissionDenied {
            Err(UpdateError::PermissionDenied {
                path: current_exe.display().to_string(),
            })
        } else {
            Err(UpdateError::Io(e))
        };
    }

    Ok(current_exe)
}

// ---------------------------------------------------------------------------
// Auto-check helpers
// ---------------------------------------------------------------------------

/// Timestamp file path for last update check.
fn last_check_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("archon").join("last_update_check"))
}

/// Returns true if enough time has passed since last check.
pub fn should_auto_check(config: &UpdateConfig) -> bool {
    if !config.auto_check {
        return false;
    }
    let Some(path) = last_check_path() else {
        return true;
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return true;
    };
    let Ok(ts) = content.trim().parse::<u64>() else {
        return true;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let interval_secs = config.check_interval_hours * 3600;
    now.saturating_sub(ts) >= interval_secs
}

pub fn record_check_time() {
    let Some(path) = last_check_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = std::fs::write(path, now.to_string());
}

// ---------------------------------------------------------------------------
// High-level entry points
// ---------------------------------------------------------------------------

/// Run the update: check, download, verify, replace.
/// Returns description of what happened.
pub async fn perform_update(config: &UpdateConfig, force: bool) -> Result<String, UpdateError> {
    let release = check_latest(config).await?;

    if !force && !is_newer(&release.version, CURRENT_VERSION) {
        return Err(UpdateError::UpToDate(format!(
            "already at latest version {}",
            CURRENT_VERSION
        )));
    }

    let binary = download_and_verify(&release).await?;
    let path = replace_binary(binary)?;

    Ok(format!(
        "Updated to {} — restart archon to use new version\nBinary: {}\n\nChangelog:\n{}",
        release.version,
        path.display(),
        // Safe truncation at char boundary
        if release.changelog.len() > 500 {
            let boundary = release
                .changelog
                .char_indices()
                .take_while(|(i, _)| *i < 500)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(500);
            &release.changelog[..boundary]
        } else {
            &release.changelog
        }
    ))
}

/// Check only — no download. Returns description of update status.
pub async fn check_update(config: &UpdateConfig) -> Result<String, UpdateError> {
    let release = check_latest(config).await?;

    if is_newer(&release.version, CURRENT_VERSION) {
        Ok(format!(
            "Update available: {} → {}\nRun 'archon update' to install",
            CURRENT_VERSION, release.version
        ))
    } else {
        Ok(format!("Up to date: {}", CURRENT_VERSION))
    }
}
