use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::spoof::{CodexManifestConfig, Manifest, SpoofError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedManifest {
    pub(crate) manifest: Manifest,
    pub(crate) fetched_at: DateTime<Utc>,
    pub(crate) fetch_url: String,
}

pub(crate) fn cache_file(config: &CodexManifestConfig) -> PathBuf {
    expand_tilde(&config.cache_dir).join("codex-compat-cache.json")
}

pub(crate) fn read_fresh_cache(
    path: &Path,
    ttl_seconds: u64,
    fetch_url: &str,
) -> Result<Option<CachedManifest>, SpoofError> {
    if !path.exists() {
        return Ok(None);
    }
    let cached = match read_cache(path) {
        Ok(cached) => cached,
        Err(err) => {
            let _ = fs::remove_file(path);
            tracing::warn!("deleted corrupt Codex manifest cache: {err}");
            return Ok(None);
        }
    };
    let age = Utc::now() - cached.fetched_at;
    if cached.fetch_url == fetch_url
        && age.num_seconds() >= 0
        && age.num_seconds() < ttl_seconds as i64
    {
        Ok(Some(cached))
    } else {
        Ok(None)
    }
}

pub(crate) fn read_cache(path: &Path) -> Result<CachedManifest, SpoofError> {
    let content = fs::read_to_string(path).map_err(|e| SpoofError::CacheCorrupt(e.to_string()))?;
    serde_json::from_str(&content).map_err(|e| SpoofError::CacheCorrupt(e.to_string()))
}

pub(crate) fn write_cache(
    path: &Path,
    manifest: &Manifest,
    fetch_url: &str,
) -> Result<(), SpoofError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| SpoofError::FetchFailed(e.to_string()))?;
    }
    let cached = CachedManifest {
        manifest: manifest.clone(),
        fetched_at: Utc::now(),
        fetch_url: fetch_url.into(),
    };
    let content = serde_json::to_string_pretty(&cached)
        .map_err(|e| SpoofError::FetchFailed(e.to_string()))?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).map_err(|e| SpoofError::FetchFailed(e.to_string()))?;
    fs::rename(&tmp, path).map_err(|e| SpoofError::FetchFailed(e.to_string()))
}

pub(crate) fn touch_cache(path: &Path) -> Result<(), SpoofError> {
    let mut cached = read_cache(path)?;
    cached.fetched_at = Utc::now();
    let content = serde_json::to_string_pretty(&cached)
        .map_err(|e| SpoofError::FetchFailed(e.to_string()))?;
    fs::write(path, content).map_err(|e| SpoofError::FetchFailed(e.to_string()))
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}
