//! PluginLoadResult — typed result struct for plugin loading.

use std::path::PathBuf;

use crate::error::PluginError;
use crate::types::PluginManifest;

// ── LoadedPlugin ──────────────────────────────────────────────────────────────

/// A plugin that has been discovered and processed (either enabled or disabled).
#[derive(Debug)]
pub struct LoadedPlugin {
    /// Plugin identifier (from manifest.name).
    pub plugin_id: String,
    /// Parsed manifest.
    pub manifest: PluginManifest,
    /// Persistent data directory for this plugin.
    pub data_dir: PathBuf,
    /// Path to the optional WASM module, if present.
    pub wasm_path: Option<PathBuf>,
}

// ── PluginLoadResult ──────────────────────────────────────────────────────────

/// Result of a full plugin directory scan.
///
/// Loading is fail-open: errors never block startup. Each plugin either loads
/// successfully (enabled/disabled) or produces a typed `PluginError` entry.
#[derive(Debug, Default)]
pub struct PluginLoadResult {
    /// Plugins that loaded successfully and are enabled.
    pub enabled: Vec<LoadedPlugin>,
    /// Plugins that loaded successfully but are disabled in config.
    pub disabled: Vec<LoadedPlugin>,
    /// Errors encountered during loading. Each entry is `(plugin_id, error)`.
    ///
    /// Plugin IDs in this list are inferred from the directory name when the
    /// manifest cannot be parsed.
    pub errors: Vec<(String, PluginError)>,
}
