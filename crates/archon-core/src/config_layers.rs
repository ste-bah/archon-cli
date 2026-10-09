//! Configuration layering system for Archon CLI.
//!
//! Supports multiple configuration layers with well-defined priority ordering:
//! User (lowest) → Project → Local → Worktree → Settings (highest).
//!
//! Each layer is a TOML file that is deep-merged on top of previous layers,
//! allowing partial overrides while inheriting unspecified keys.

use std::fs;
use std::path::{Path, PathBuf};

use toml::Value;

use crate::config::{ArchonConfig, ConfigError, validate};

// ---------------------------------------------------------------------------
// ConfigLayer enum
// ---------------------------------------------------------------------------

/// Identifies which configuration layer a value originated from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigLayer {
    /// User-level config, e.g. `~/.config/archon/config.toml`
    User,
    /// Project-level config: `{work_dir}/.archon/config.toml`
    Project,
    /// Local (gitignored) project config: `{work_dir}/.archon/config.local.toml`
    Local,
    /// Worktree-specific config (reserved for future use)
    Worktree,
    /// Explicit settings overlay passed via `--settings` flag
    Settings,
}

impl std::fmt::Display for ConfigLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigLayer::User => write!(f, "user"),
            ConfigLayer::Project => write!(f, "project"),
            ConfigLayer::Local => write!(f, "local"),
            ConfigLayer::Worktree => write!(f, "worktree"),
            ConfigLayer::Settings => write!(f, "settings"),
        }
    }
}

// ---------------------------------------------------------------------------
// LayerInfo
// ---------------------------------------------------------------------------

/// Describes a discovered configuration layer and its file path.
#[derive(Debug)]
pub struct LayerInfo {
    /// Which layer this entry represents.
    pub layer: ConfigLayer,
    /// Absolute path to the configuration file.
    pub path: PathBuf,
}

// ---------------------------------------------------------------------------
// deep_merge_toml
// ---------------------------------------------------------------------------

/// Recursively deep-merge two TOML values.
///
/// - If both values are tables, keys are merged recursively: overlay keys
///   override base keys, missing keys inherit from base.
/// - Arrays are **replaced** (overlay wins entirely), not appended.
/// - Scalars: overlay wins.
pub fn deep_merge_toml(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Table(mut base_map), Value::Table(overlay_map)) => {
            for (key, overlay_val) in overlay_map {
                let merged = match base_map.remove(&key) {
                    Some(base_val) => deep_merge_toml(base_val, overlay_val),
                    None => overlay_val,
                };
                base_map.insert(key, merged);
            }
            Value::Table(base_map)
        }
        // For all non-table cases (including arrays), overlay wins.
        (_base, overlay) => overlay,
    }
}

// ---------------------------------------------------------------------------
// discover_config_paths
// ---------------------------------------------------------------------------

/// Discover which configuration layer files exist on disk.
///
/// Returns a `Vec<LayerInfo>` in priority order (lowest priority first):
/// User → Project → Local → Worktree.
///
/// Only layers whose files actually exist are included.
pub fn discover_config_paths(
    user_config: Option<&Path>,
    work_dir: &Path,
    worktree_name: Option<&str>,
) -> Vec<LayerInfo> {
    let mut layers = Vec::new();

    // User layer
    if let Some(path) = user_config
        && path.exists()
    {
        layers.push(LayerInfo {
            layer: ConfigLayer::User,
            path: path.to_path_buf(),
        });
    }

    // Project layer: {work_dir}/.archon/config.toml
    let project_path = work_dir.join(".archon").join("config.toml");
    if project_path.exists() {
        layers.push(LayerInfo {
            layer: ConfigLayer::Project,
            path: project_path,
        });
    }

    // Local layer: {work_dir}/.archon/config.local.toml
    let local_path = work_dir.join(".archon").join("config.local.toml");
    if local_path.exists() {
        layers.push(LayerInfo {
            layer: ConfigLayer::Local,
            path: local_path,
        });
    }

    // Worktree layer (placeholder — check for worktree-specific config)
    if let Some(name) = worktree_name {
        let worktree_path = work_dir
            .join(".archon")
            .join("worktrees")
            .join(format!("{name}.toml"));
        if worktree_path.exists() {
            layers.push(LayerInfo {
                layer: ConfigLayer::Worktree,
                path: worktree_path,
            });
        }
    }

    layers
}

// ---------------------------------------------------------------------------
// load_layered_config
// ---------------------------------------------------------------------------

/// Load configuration by merging multiple TOML layers.
///
/// Layers are merged in priority order (lowest first): User → Project →
/// Local → Worktree. An optional `settings_path` overlay is applied last
/// (highest priority). The `filter` parameter, when `Some`, restricts which
/// layers are included.
///
/// Errors in selected layers propagate. Excluded layers are never read.
pub fn load_layered_config(
    user_config: Option<&Path>,
    work_dir: &Path,
    settings_path: Option<&Path>,
    filter: Option<&[ConfigLayer]>,
) -> Result<ArchonConfig, ConfigError> {
    let discovered = discover_config_paths(user_config, work_dir, None);

    // Filter layers if requested
    let active_layers: Vec<&LayerInfo> = discovered
        .iter()
        .filter(|info| match &filter {
            Some(allowed) => allowed.contains(&info.layer),
            None => true,
        })
        .collect();

    // Start with an empty table and merge each layer on top
    let mut merged = Value::Table(toml::map::Map::new());
    let mut audit_sources = std::collections::BTreeMap::new();

    // Every selected layer is authority. Discarding a parse failure could
    // silently erase an environment allowlist and launch with another policy.
    for info in &active_layers {
        let value = read_and_parse_toml(&info.path)?;
        validate_audit_layer(&info.path, &value)?;
        crate::config::record_audit_sources(
            &value,
            &info.path,
            &info.layer.to_string(),
            &mut audit_sources,
        );
        merged = deep_merge_toml(merged, value);
    }

    // An explicitly selected overlay must exist and parse; no silent fallback.
    if let Some(path) = settings_path {
        let value = read_and_parse_toml(path)?;
        validate_audit_layer(path, &value)?;
        crate::config::record_audit_sources(&value, path, "settings", &mut audit_sources);
        merged = deep_merge_toml(merged, value);
    }

    // Deserialize the merged value into ArchonConfig
    let mut config: ArchonConfig = merged
        .try_into()
        .map_err(|e: toml::de::Error| ConfigError::ParseError(e))?;

    config.workflow.repository_audit.sources = audit_sources;
    validate(&config)?;
    for warning in crate::config::write_call_time_budget_warnings(&config) {
        eprintln!("warning: {warning}");
    }
    Ok(config)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read a file and parse it as a TOML `Value`.
fn read_and_parse_toml(path: &Path) -> Result<Value, ConfigError> {
    let content = fs::read_to_string(path).map_err(|error| {
        ConfigError::ValidationError(format!(
            "configuration at {} could not be read: {error}",
            path.display()
        ))
    })?;
    content.parse().map_err(|error: toml::de::Error| {
        ConfigError::ValidationError(format!(
            "configuration at {} could not be parsed: {error}",
            path.display()
        ))
    })
}

fn validate_audit_layer(path: &Path, value: &Value) -> Result<(), ConfigError> {
    if let Some(audit) = value
        .get("workflow")
        .and_then(|workflow| workflow.get("repository_audit"))
    {
        let _: crate::config::RepositoryAuditConfig =
            audit.clone().try_into().map_err(|error: toml::de::Error| {
                ConfigError::ValidationError(format!(
                    "workflow.repository_audit in {}: {error}",
                    path.display()
                ))
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod strict_loading_tests {
    use super::*;

    #[test]
    fn explicit_missing_settings_layer_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing.toml");
        let error = load_layered_config(None, root.path(), Some(&missing), Some(&[]))
            .expect_err("an explicit settings path must exist");
        assert!(error.to_string().contains("could not be read"));
    }

    #[test]
    fn malformed_selected_settings_layer_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let settings = root.path().join("settings.toml");
        fs::write(&settings, "[workflow\ninvalid = true").unwrap();
        let error = load_layered_config(None, root.path(), Some(&settings), Some(&[]))
            .expect_err("a malformed selected layer must not be skipped");
        assert!(error.to_string().contains("could not be parsed"));
    }

    #[test]
    fn malformed_selected_user_layer_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user.toml");
        fs::write(&user, "[workflow\ninvalid = true").unwrap();
        let error = load_layered_config(Some(&user), root.path(), None, Some(&[ConfigLayer::User]))
            .expect_err("a malformed selected user layer must not be skipped");
        assert!(error.to_string().contains("could not be parsed"));
    }
}
