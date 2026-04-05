//! Plugin metadata and manifest types for TASK-CLI-301.

use serde::{Deserialize, Serialize};

// ── PluginManifest ────────────────────────────────────────────────────────────

/// Parsed contents of `.claude-plugin/plugin.json`.
///
/// This is the JSON manifest format (not TOML). Fields are snake_case on-disk
/// to match common JSON conventions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Plugin identifier (required).
    pub name: String,
    /// Semver version string (required).
    pub version: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// Plugin author.
    #[serde(default)]
    pub author: Option<String>,
    /// Project homepage URL.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Source repository URL.
    #[serde(default)]
    pub repository: Option<String>,
    /// SPDX license identifier.
    #[serde(default)]
    pub license: Option<String>,
    /// Searchable keywords.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Required plugin dependencies (`"name@version"` strings).
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Capabilities the plugin requires (as string names, validated at load time).
    #[serde(default)]
    pub capabilities: Vec<String>,
}

// ── PluginMetadata ────────────────────────────────────────────────────────────

/// Runtime metadata derived from a loaded plugin manifest.
#[derive(Debug, Clone)]
pub struct PluginMetadata {
    /// Plugin name.
    pub name: String,
    /// Plugin version.
    pub version: String,
    /// Optional description.
    pub description: Option<String>,
    /// Dependency list.
    pub dependencies: Vec<String>,
}

impl PluginMetadata {
    /// Derive runtime metadata from a parsed manifest.
    pub fn from_manifest(manifest: &PluginManifest) -> Self {
        Self {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            description: manifest.description.clone(),
            dependencies: manifest.dependencies.clone(),
        }
    }
}

// ── PluginConfig ──────────────────────────────────────────────────────────────

/// Runtime configuration for a loaded plugin instance.
#[derive(Debug, Clone)]
pub struct PluginConfig {
    /// Maximum memory allowed for this plugin (bytes). Default: 64 MiB.
    pub max_memory_bytes: usize,
    /// Fuel budget per host function call. Default: 10M.
    pub fuel_budget: u64,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            max_memory_bytes: crate::abi::DEFAULT_MAX_MEMORY_BYTES,
            fuel_budget: crate::abi::DEFAULT_FUEL_BUDGET,
        }
    }
}
