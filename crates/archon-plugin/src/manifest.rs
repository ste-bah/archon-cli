//! Manifest loading and validation for TASK-CLI-303.
//!
//! Loads `.archon-plugin/plugin.json` from a plugin directory,
//! parses it as `PluginManifest`, and validates required fields.

use std::path::Path;

use crate::error::PluginError;
use crate::types::{ManifestCapability, PluginManifest};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Relative path from a plugin directory to its manifest.
pub const MANIFEST_RELATIVE_PATH: &str = ".archon-plugin/plugin.json";
const SUPPORTED_HOST_FUNCTIONS: &[&str] = &[
    "archon_log",
    "archon_register_tool",
    "archon_register_hook",
    "archon_register_command",
    "archon_host_call",
    "archon_api_version",
];

// ── Loading ───────────────────────────────────────────────────────────────────

/// Load and parse a `PluginManifest` from `<plugin_dir>/.archon-plugin/plugin.json`.
///
/// Returns `None` if the manifest file does not exist (silent skip).
/// Returns `Err` with `ManifestParseError` on JSON parse failure.
/// Returns `Err` with `ManifestValidationError` on field validation failure.
pub fn load_manifest(plugin_dir: &Path) -> Option<Result<PluginManifest, PluginError>> {
    let manifest_path = plugin_dir.join(MANIFEST_RELATIVE_PATH);
    let manifest_path = if manifest_path.exists() {
        manifest_path
    } else {
        let old_path = plugin_dir.join(".claude-plugin/plugin.json");
        if old_path.exists() {
            tracing::warn!(
                "Loading from deprecated path {}. Rename to {} to suppress this warning.",
                old_path.display(),
                manifest_path.display()
            );
            old_path
        } else {
            return None;
        }
    };

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
            fields: vec![format!("name '{}' must not contain spaces", manifest.name)],
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

    let manifest_path = plugin_dir.join(MANIFEST_RELATIVE_PATH);
    let mut capability_errors = Vec::new();
    for capability in &manifest.capabilities {
        validate_capability(capability, &mut capability_errors);
    }
    for required in &manifest.required_host_functions {
        if !SUPPORTED_HOST_FUNCTIONS.contains(&required.as_str()) {
            capability_errors.push(format!(
                "required_host_functions '{}' is not implemented by the host",
                required
            ));
        }
    }
    if !capability_errors.is_empty() {
        return Err(PluginError::ManifestValidationError {
            path: manifest_path,
            fields: capability_errors,
        });
    }

    Ok(manifest)
}

fn validate_capability(capability: &ManifestCapability, errors: &mut Vec<String>) {
    match capability {
        ManifestCapability::Legacy(kind) => errors.push(format!(
            "capabilities entry '{}' uses the legacy string form; migrate to {{\"kind\":\"{}\", ...}}",
            kind, kind
        )),
        ManifestCapability::Structured(cap) => match cap.kind.as_str() {
            "ReadFs" | "WriteFs" => {
                if cap.paths.is_empty() {
                    errors.push(format!("{} requires a non-empty paths array", cap.kind));
                }
                for path in &cap.paths {
                    if path.as_os_str().is_empty() || !path.is_absolute() {
                        errors.push(format!(
                            "{} path '{}' must be absolute",
                            cap.kind,
                            path.display()
                        ));
                    }
                }
                if !cap.hosts.is_empty() {
                    errors.push(format!("{} does not accept hosts", cap.kind));
                }
            }
            "Network" => {
                if cap.hosts.is_empty() {
                    errors.push("Network requires a non-empty hosts array".to_string());
                }
                if !cap.paths.is_empty() {
                    errors.push("Network does not accept paths".to_string());
                }
                for host in &cap.hosts {
                    if host.trim().is_empty() {
                        errors.push("Network host entries must not be empty".to_string());
                    } else if host == "*" {
                        errors.push(
                            "Network host '*' is not allowed in plugin manifests; enumerate hosts"
                                .to_string(),
                        );
                    }
                }
            }
            "ToolRegister" | "HookRegister" | "CommandRegister" | "LspRegister"
            | "DataDirWrite" => {
                if !cap.paths.is_empty() || !cap.hosts.is_empty() {
                    errors.push(format!("{} does not accept paths or hosts", cap.kind));
                }
            }
            other => errors.push(format!("unknown capability kind '{}'", other)),
        },
    }
}
