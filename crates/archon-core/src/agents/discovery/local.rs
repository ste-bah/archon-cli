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

/// Extract YAML frontmatter from a markdown file.
///
/// Frontmatter is delimited by `---` lines. Returns the YAML string
/// between the first and second `---` delimiters, or None if the
/// file does not start with `---`.
fn extract_yaml_frontmatter(content: &str) -> Option<String> {
    let mut lines = content.lines();
    // First line must be "---"
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut frontmatter = String::new();
    for line in lines {
        if line.trim() == "---" {
            return Some(frontmatter);
        }
        frontmatter.push_str(line);
        frontmatter.push('\n');
    }
    None // No closing "---"
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

        let parsed: Vec<Option<AgentMetadata>> =
            files.par_iter().map(|f| self.parse_one(f)).collect();

        let mut loaded = 0;
        let mut invalid = 0;

        for opt in parsed {
            let meta = match opt {
                Some(m) => m,
                None => continue, // no frontmatter .md file, silently skipped
            };
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
    /// Returns None for files that should be silently skipped
    /// (e.g. .md files without YAML frontmatter like READMEs).
    /// On parse/validation failure, returns Some with state=Invalid.
    fn parse_one(&self, file: &DiscoveredFile) -> Option<AgentMetadata> {
        match self.try_parse(file) {
            Ok(meta) => Some(meta),
            Err(reason) => {
                // "no YAML frontmatter" means a .md file without --- delimiters
                // (e.g. README) — skip silently, not an agent file
                if reason == "no YAML frontmatter" {
                    debug!(path = ?file.path, "skipping .md file without frontmatter");
                    return None;
                }
                // EC-DISCOVERY-001: invalid files preserved with state=Invalid
                let name = file
                    .path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                warn!(path = ?file.path, reason = %reason, "invalid agent file");
                Some(AgentMetadata {
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
                })
            }
        }
    }

    fn try_parse(&self, file: &DiscoveredFile) -> Result<AgentMetadata, String> {
        let content =
            std::fs::read_to_string(&file.path).map_err(|e| format!("read error: {e}"))?;

        let ext = file.path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let mut value: serde_json::Value = match ext {
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
            "md" => {
                let frontmatter = extract_yaml_frontmatter(&content)
                    .ok_or_else(|| "no YAML frontmatter".to_string())?;
                serde_yml::from_str(&frontmatter)
                    .map_err(|e| format!("YAML frontmatter parse error: {e}"))?
            }
            _ => return Err(format!("unsupported extension: {ext}")),
        };

        // Flat-file agents often lack version and resource_requirements.
        // Inject defaults so schema validation passes.
        if ext == "md" {
            if value.get("version").is_none() {
                value["version"] = serde_json::json!("0.1.0");
            }
            if value.get("resource_requirements").is_none() {
                value["resource_requirements"] = serde_json::json!({
                    "cpu": 1.0,
                    "memory_mb": 0,
                    "timeout_sec": 300
                });
            }
        }

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::catalog::DiscoveryCatalog;
    use std::fs;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // extract_yaml_frontmatter tests
    // -----------------------------------------------------------------------

    #[test]
    fn extract_valid_frontmatter() {
        let content = "---\nname: test\nversion: 1.0.0\ndescription: hi\nresource_requirements:\n  cpu: 1.0\n  memory_mb: 512\n  timeout_sec: 300\n---\n# Body\n";
        let fm = extract_yaml_frontmatter(content).unwrap();
        assert!(fm.contains("name: test"));
        assert!(fm.contains("version: 1.0.0"));
    }

    #[test]
    fn extract_no_frontmatter_returns_none() {
        assert!(extract_yaml_frontmatter("# Just a README\n\nNo frontmatter here\n").is_none());
        assert!(extract_yaml_frontmatter("").is_none());
    }

    #[test]
    fn extract_unclosed_frontmatter_returns_none() {
        let content = "---\nname: test\n";
        assert!(extract_yaml_frontmatter(content).is_none());
    }

    // -----------------------------------------------------------------------
    // Flat-file agent loading tests
    // -----------------------------------------------------------------------

    fn setup_mixed_fixture(tmp: &TempDir) {
        let root = tmp.path();

        // 1. Valid flat-file agent (core/test-coder.md)
        fs::create_dir_all(root.join("core")).unwrap();
        fs::write(
            root.join("core/test-coder.md"),
            "---\nname: test-coder\ndescription: A test coder agent\ncapabilities:\n  - code_generation\n  - refactoring\ntags:\n  - rust\n  - test\n---\n\n# Test Coder\n\nI write code.\n",
        )
        .unwrap();

        // 2. Invalid flat-file (malformed YAML in frontmatter)
        fs::write(
            root.join("core/bad-agent.md"),
            "---\nname: [bad:::yaml\ndescription:\n---\n\nOops\n",
        )
        .unwrap();

        // 3. Plain .md file with no frontmatter (README-like)
        fs::write(
            root.join("core/README.md"),
            "# Core Agents\n\nThis directory contains core agents.\n",
        )
        .unwrap();
    }

    #[test]
    fn load_all_finds_flat_file_agent() {
        let tmp = TempDir::new().unwrap();
        setup_mixed_fixture(&tmp);

        let validator = Arc::new(AgentSchemaValidator::new().unwrap());
        let source = LocalDiscoverySource::new(tmp.path().to_path_buf(), validator);
        let catalog = DiscoveryCatalog::new();

        let report = source.load_all(&catalog).unwrap();
        // 1 valid flat-file, 1 invalid flat-file
        assert_eq!(report.loaded, 1, "expected 1 loaded (flat-file valid)");
        assert_eq!(
            report.invalid, 1,
            "expected 1 invalid (bad YAML frontmatter)"
        );
    }

    #[test]
    fn malformed_frontmatter_becomes_invalid() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("test")).unwrap();
        fs::write(
            tmp.path().join("test/bad.md"),
            "---\nname: [bad:::yaml\ndescription:\n---\n",
        )
        .unwrap();

        let validator = Arc::new(AgentSchemaValidator::new().unwrap());
        let source = LocalDiscoverySource::new(tmp.path().to_path_buf(), validator);
        let catalog = DiscoveryCatalog::new();

        let report = source.load_all(&catalog).unwrap();
        assert_eq!(report.loaded, 0);
        assert_eq!(report.invalid, 1);
    }

    #[test]
    fn no_frontmatter_md_skipped_not_invalid() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("test")).unwrap();
        fs::write(
            tmp.path().join("test/README.md"),
            "# README\n\nNot an agent.\n",
        )
        .unwrap();

        let validator = Arc::new(AgentSchemaValidator::new().unwrap());
        let source = LocalDiscoverySource::new(tmp.path().to_path_buf(), validator);
        let catalog = DiscoveryCatalog::new();

        let report = source.load_all(&catalog).unwrap();
        assert_eq!(report.loaded, 0, "README should not be loaded");
        assert_eq!(report.invalid, 0, "README should not be marked invalid");
    }

    #[test]
    fn flat_file_agent_maps_fields_correctly() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("core")).unwrap();
        fs::write(
            tmp.path().join("core/test-coder.md"),
            "---\nname: test-coder\ndescription: A test coder\ntags:\n  - rust\ncapabilities:\n  - code_generation\n---\n\n# Body\n",
        )
        .unwrap();

        let validator = Arc::new(AgentSchemaValidator::new().unwrap());
        let source = LocalDiscoverySource::new(tmp.path().to_path_buf(), validator);
        let catalog = DiscoveryCatalog::new();

        source.load_all(&catalog).unwrap();

        // Agent is stored with key (name, version)
        let key: crate::agents::catalog::AgentKey =
            ("test-coder".to_string(), semver::Version::new(0, 1, 0));
        let agent = catalog.get(&key).expect("test-coder should be in catalog");
        assert_eq!(agent.name, "test-coder");
        assert_eq!(agent.description, "A test coder");
        assert!(agent.tags.contains(&"rust".to_string()));
        assert!(agent.capabilities.contains(&"code_generation".to_string()));
        assert_eq!(agent.category, "core");
        assert_eq!(agent.state, AgentState::Valid);
        // Defaults injected for missing required fields
        assert_eq!(agent.version, semver::Version::new(0, 1, 0));
    }
}
