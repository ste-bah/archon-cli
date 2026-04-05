//! Pre-compiled WASM module cache for TASK-CLI-303.
//!
//! Cache layout: `<cache_dir>/<sanitized-plugin-id>/<version>/module.bin`

use std::path::PathBuf;

use crate::error::PluginError;

// ── WasmCache ─────────────────────────────────────────────────────────────────

/// Pre-compiled WASM module cache.
///
/// Caches compiled module bytes at `<cache_dir>/<plugin_id>/<version>/module.bin`.
pub struct WasmCache {
    cache_dir: PathBuf,
}

impl WasmCache {
    /// Create a cache rooted at `cache_dir`.
    ///
    /// The directory is created on first `store()` call, not during construction.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// Return the path for a cached module, sanitizing `plugin_id` and `version`.
    fn module_path(&self, plugin_id: &str, version: &str) -> PathBuf {
        let safe_id = sanitize(plugin_id);
        let safe_ver = sanitize(version);
        self.cache_dir
            .join(safe_id)
            .join(safe_ver)
            .join("module.bin")
    }

    /// Retrieve cached bytes for `plugin_id` at `version`, if present.
    pub fn get(&self, plugin_id: &str, version: &str) -> Option<Vec<u8>> {
        let path = self.module_path(plugin_id, version);
        std::fs::read(path).ok()
    }

    /// Store `bytes` in the cache for `plugin_id` at `version`.
    pub fn store(&self, plugin_id: &str, version: &str, bytes: &[u8]) -> Result<(), PluginError> {
        let path = self.module_path(plugin_id, version);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| PluginError::LoadFailed(format!("cache dir: {e}")))?;
        }
        std::fs::write(&path, bytes)
            .map_err(|e| PluginError::LoadFailed(format!("cache write: {e}")))?;
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Replace filesystem-unsafe characters with underscores.
///
/// Segments consisting entirely of dots (e.g. `..`) are replaced to prevent
/// path traversal attacks.
fn sanitize(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Reject pure-dot segments like "." or ".."
    if s.chars().all(|c| c == '.') {
        return "_".to_string();
    }
    s
}
