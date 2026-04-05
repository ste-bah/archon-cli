//! Manifest loading and validation for TASK-CLI-303.
//!
//! Loads `.claude-plugin/plugin.json` from a plugin directory,
//! parses it as `PluginManifest`, and validates required fields.

use std::path::Path;

use crate::error::PluginError;
use crate::types::PluginManifest;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Relative path from a plugin directory to its manifest.
pub const MANIFEST_RELATIVE_PATH: &str = ".claude-plugin/plugin.json";

// ── Loading ───────────────────────────────────────────────────────────────────

/// Load and parse a `PluginManifest` from `<plugin_dir>/.claude-plugin/plugin.json`.
///
/// Returns `None` if the manifest file does not exist (silent skip).
/// Returns `Err` with `ManifestParseError` on JSON parse failure.
/// Returns `Err` with `ManifestValidationError` on field validation failure.
pub fn load_manifest(plugin_dir: &Path) -> Option<Result<PluginManifest, PluginError>> {
    let manifest_path = plugin_dir.join(MANIFEST_RELATIVE_PATH);
    if !manifest_path.exists() {
        return None;
    }

    let contents = match std::fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(e) => {
            return Some(Err(PluginError::ManifestParseError {
                path: manifest_path,
                reason: e.to_string(),
            }));
        }
    };

    let manifest: PluginManifest = match serde_json::from_str(&contents) {
        Ok(m) => m,
        Err(e) => {
            return Some(Err(PluginError::ManifestParseError {
                path: manifest_path,
                reason: e.to_string(),
            }));
        }
    };

    Some(validate_manifest(manifest, plugin_dir))
}

// ── Validation ────────────────────────────────────────────────────────────────

/// Validate a parsed `PluginManifest` for required fields and field constraints.
///
/// - `name`: required, non-empty, no spaces
/// - `version`: required, non-empty
pub fn validate_manifest(
    manifest: PluginManifest,
    plugin_dir: &Path,
) -> Result<PluginManifest, PluginError> {
    let mut missing = Vec::new();

    if manifest.name.is_empty() {
        missing.push("name".to_string());
    } else if manifest.name.contains(' ') {
        return Err(PluginError::ManifestValidationError {
            path: plugin_dir.join(MANIFEST_RELATIVE_PATH),
            fields: vec![format!(
                "name '{}' must not contain spaces",
                manifest.name
            )],
        });
    }

    if manifest.version.is_empty() {
        missing.push("version".to_string());
    }

    if !missing.is_empty() {
        return Err(PluginError::ManifestValidationError {
            path: plugin_dir.join(MANIFEST_RELATIVE_PATH),
            fields: missing,
        });
    }

    Ok(manifest)
}
