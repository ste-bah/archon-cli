use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::index::Symbol;

/// Serialisable snapshot of the codebase index stored on disk.
#[derive(Serialize, Deserialize)]
pub struct CachedIndex {
    pub symbols: HashMap<String, Vec<Symbol>>,
    pub mtimes: HashMap<String, u64>,
}

/// Compute the cache-file path for a given project root path.
///
/// Format: `~/.local/share/archon/cartographer/<hash>.json`
/// where `<hash>` is a `DefaultHasher` hash of the canonical project path string.
pub fn cache_path(project_path: &Path) -> Option<PathBuf> {
    let base = dirs::data_local_dir()?;
    let cache_dir = base.join("archon").join("cartographer");

    let canonical = project_path.to_string_lossy();
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    let hash = hasher.finish();

    Some(cache_dir.join(format!("{hash}.json")))
}

/// Load a `CachedIndex` from disk, returning `None` if missing or corrupt.
pub fn load_cache(project_path: &Path) -> Option<CachedIndex> {
    let path = cache_path(project_path)?;
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Persist a `CachedIndex` to disk.
///
/// Creates parent directories as needed.  Silently ignores write failures.
pub fn save_cache(project_path: &Path, index: &CachedIndex) {
    let path = match cache_path(project_path) {
        Some(p) => p,
        None => {
            tracing::warn!("Could not determine cache path for cartographer");
            return;
        }
    };

    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("Failed to create cartographer cache dir: {e}");
            return;
        }

    match serde_json::to_string_pretty(index) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                tracing::warn!("Failed to write cartographer cache: {e}");
            }
        }
        Err(e) => {
            tracing::warn!("Failed to serialise cartographer cache: {e}");
        }
    }
}
