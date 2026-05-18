//! Plugin metadata and manifest types for TASK-CLI-301.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── PluginManifest ────────────────────────────────────────────────────────────

/// Parsed contents of `.archon-plugin/plugin.json`.
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
    /// Capabilities the plugin requires, validated at load time.
    #[serde(default)]
    pub capabilities: Vec<ManifestCapability>,
    /// Host functions this plugin requires from the Archon WASM host.
    #[serde(default)]
    pub required_host_functions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ManifestCapability {
    /// Legacy capability declarations such as `"ReadFs"`.
    ///
    /// These still deserialize so the loader can return a migration error
    /// instead of a vague JSON parse error, but validation rejects them.
    Legacy(String),
    Structured(StructuredCapability),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredCapability {
    pub kind: String,
    #[serde(default)]
    pub paths: Vec<PathBuf>,
    #[serde(default)]
    pub hosts: Vec<String>,
}

impl ManifestCapability {
    pub fn kind(&self) -> &str {
        match self {
            ManifestCapability::Legacy(kind) => kind,
            ManifestCapability::Structured(cap) => cap.kind.as_str(),
        }
    }

    pub fn label(&self) -> String {
        match self {
            ManifestCapability::Legacy(kind) => format!("{kind} (legacy)"),
            ManifestCapability::Structured(cap) => match cap.kind.as_str() {
                "ReadFs" | "WriteFs" => {
                    let paths = cap
                        .paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{} paths=[{}]", cap.kind, paths)
                }
                "Network" => format!("Network hosts=[{}]", cap.hosts.join(", ")),
                kind => kind.to_string(),
            },
        }
    }
}

impl PluginManifest {
    pub fn capability_labels(&self) -> Vec<String> {
        self.capabilities
            .iter()
            .map(ManifestCapability::label)
            .collect()
    }
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
