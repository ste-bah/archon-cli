// TASK-AGS-303: LocalDiscoverySource — parses discovered files into
// AgentMetadata, validates against schema, and inserts into the catalog.
// Uses rayon par_iter for parallel parsing (NFR-PERF-002: <1s for 234+ agents).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use rayon::prelude::*;
use tracing::{debug, warn};

use crate::agents::catalog::{DiscoveryCatalog, DiscoveryError};
use crate::agents::discovery::walker::{DiscoveredFile, walk_agents_dir};
use crate::agents::metadata::{AgentMetadata, AgentState, ResourceReq, SourceKind};
use crate::agents::schema::AgentSchemaValidator;

/// Report from a local discovery load.
#[derive(Debug)]
pub struct LoadReport {
    pub loaded: usize,
    pub invalid: usize,
    pub duration_ms: u128,
}

/// Loads agent metadata from a local directory tree.
pub struct LocalDiscoverySource {
    root: PathBuf,
    validator: Arc<AgentSchemaValidator>,
}

impl LocalDiscoverySource {
    pub fn new(root: PathBuf, validator: Arc<AgentSchemaValidator>) -> Self {
        Self { root, validator }
    }

    /// Walk the directory, parse all files in parallel, insert into catalog.
    /// Invalid files become AgentState::Invalid(reason) per EC-DISCOVERY-001.
    pub fn load_all(&self, catalog: &DiscoveryCatalog) -> Result<LoadReport, DiscoveryError> {
        let start = std::time::Instant::now();

        let files = walk_agents_dir(&self.root)?;
        debug!(count = files.len(), root = ?self.root, "discovered agent files");

        let parsed: Vec<AgentMetadata> = files.par_iter().map(|f| self.parse_one(f)).collect();

        let mut loaded = 0;
        let mut invalid = 0;

        for meta in parsed {
            match &meta.state {
                AgentState::Valid => loaded += 1,
                AgentState::Invalid(_) => invalid += 1,
                AgentState::Stale => loaded += 1,
            }
            if let Err(e) = catalog.insert(meta) {
                warn!("failed to insert agent: {e}");
            }
        }

        Ok(LoadReport {
            loaded,
            invalid,
            duration_ms: start.elapsed().as_millis(),
        })
    }

    /// Parse a single discovered file into AgentMetadata.
    /// On parse/validation failure, returns metadata with state=Invalid.
    fn parse_one(&self, file: &DiscoveredFile) -> AgentMetadata {
        match self.try_parse(file) {
            Ok(meta) => meta,
            Err(reason) => {
                // EC-DISCOVERY-001: invalid files preserved with state=Invalid
                let name = file
                    .path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                warn!(path = ?file.path, reason = %reason, "invalid agent file");
                AgentMetadata {
                    name,
                    version: semver::Version::new(0, 0, 0),
                    description: format!("Invalid: {reason}"),
                    category: file.category.clone(),
                    tags: vec![],
                    capabilities: vec![],
                    input_schema: serde_json::json!({}),
                    output_schema: serde_json::json!({}),
                    resource_requirements: ResourceReq::default(),
                    dependencies: vec![],
                    source_path: file.path.clone(),
                    source_kind: SourceKind::Local,
                    state: AgentState::Invalid(reason),
                    loaded_at: Utc::now(),
                }
            }
        }
    }

    fn try_parse(&self, file: &DiscoveredFile) -> Result<AgentMetadata, String> {
        let content =
            std::fs::read_to_string(&file.path).map_err(|e| format!("read error: {e}"))?;

        let ext = file.path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let value: serde_json::Value = match ext {
            "json" => {
                serde_json::from_str(&content).map_err(|e| format!("JSON parse error: {e}"))?
            }
            "yaml" | "yml" => {
                serde_yml::from_str(&content).map_err(|e| format!("YAML parse error: {e}"))?
            }
            "toml" => {
                let toml_val: toml::Value =
                    toml::from_str(&content).map_err(|e| format!("TOML parse error: {e}"))?;
                serde_json::to_value(toml_val)
                    .map_err(|e| format!("TOML->JSON conversion error: {e}"))?
            }
            _ => return Err(format!("unsupported extension: {ext}")),
        };

        // Validate against canonical schema
        if let Err(report) = self.validator.validate(&value) {
            return Err(report.reason());
        }

        // Parse version string as SemVer
        let version_str = value["version"].as_str().ok_or("version is not a string")?;
        let version =
            semver::Version::parse(version_str).map_err(|e| format!("invalid SemVer: {e}"))?;

        Ok(AgentMetadata {
            name: value["name"].as_str().unwrap_or("").to_string(),
            version,
            description: value["description"].as_str().unwrap_or("").to_string(),
            category: file.category.clone(),
            tags: value["tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            capabilities: value["capabilities"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            input_schema: value
                .get("input_schema")
                .cloned()
                .unwrap_or(serde_json::json!({})),
            output_schema: value
                .get("output_schema")
                .cloned()
                .unwrap_or(serde_json::json!({})),
            resource_requirements: serde_json::from_value(
                value
                    .get("resource_requirements")
                    .cloned()
                    .unwrap_or(serde_json::json!({})),
            )
            .unwrap_or_default(),
            dependencies: value
                .get("dependencies")
                .and_then(|d| serde_json::from_value(d.clone()).ok())
                .unwrap_or_default(),
            source_path: file.path.clone(),
            source_kind: SourceKind::Local,
            state: AgentState::Valid,
            loaded_at: Utc::now(),
        })
    }

    /// The root directory this source scans.
    pub fn root(&self) -> &Path {
        &self.root
    }
}
