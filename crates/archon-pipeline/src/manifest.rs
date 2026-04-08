//! Pipeline manifest parser — reads `pipeline.toml` files that define sequential
//! execution order, phase groupings, and critical (hard-gate) agents.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

// ── Types ────────────────────────────────────────────────────────────────────

/// Top-level manifest parsed from a `pipeline.toml` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineManifest {
    pub pipeline: PipelineMeta,
    #[serde(default)]
    pub defaults: ManifestDefaults,
    pub phases: Vec<PhaseDefinition>,
    #[serde(rename = "agent")]
    pub agents: Vec<ManifestAgent>,
}

/// Metadata about the pipeline itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineMeta {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub context_window: Option<u32>,
}

/// Optional default values inherited by agents unless overridden.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestDefaults {
    #[serde(default)]
    pub algorithm: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// A named phase that groups agents into logical stages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseDefinition {
    pub id: u8,
    pub name: String,
    #[serde(default)]
    pub tool_access: Option<String>,
}

/// An agent entry inside the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestAgent {
    pub key: String,
    pub phase: u8,
    #[serde(default)]
    pub critical: bool,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Load and validate a pipeline manifest from a TOML file on disk.
pub fn load_manifest(path: &Path) -> Result<PipelineManifest> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read manifest file: {}", path.display()))?;
    parse_manifest(&content)
}

/// Cross-reference manifest agents against a list of known agent keys.
///
/// Returns a (possibly empty) list of human-readable warning strings:
/// - agents declared in the manifest but missing from `agent_keys`
/// - keys present in `agent_keys` but not referenced in the manifest
pub fn validate(manifest: &PipelineManifest, agent_keys: &[String]) -> Vec<String> {
    let mut warnings = Vec::new();

    let manifest_keys: HashSet<&str> = manifest.agents.iter().map(|a| a.key.as_str()).collect();
    let known_keys: HashSet<&str> = agent_keys.iter().map(|k| k.as_str()).collect();

    // Agents in manifest but missing from known keys
    for agent in &manifest.agents {
        if !known_keys.contains(agent.key.as_str()) {
            warnings.push(format!(
                "agent '{}' declared in manifest but has no matching agent file",
                agent.key
            ));
        }
    }

    // Known keys not referenced in the manifest
    for key in agent_keys {
        if !manifest_keys.contains(key.as_str()) {
            warnings.push(format!(
                "agent file '{}' exists but is not referenced in the manifest",
                key
            ));
        }
    }

    warnings
}

// ── Internal ─────────────────────────────────────────────────────────────────

/// Parse a TOML string into a validated `PipelineManifest`.
fn parse_manifest(toml_str: &str) -> Result<PipelineManifest> {
    let manifest: PipelineManifest =
        toml::from_str(toml_str).context("failed to parse pipeline manifest TOML")?;

    validate_phases(&manifest.phases)?;
    validate_unique_agent_keys(&manifest.agents)?;

    Ok(manifest)
}

/// Ensure phase IDs are contiguous starting from 1.
fn validate_phases(phases: &[PhaseDefinition]) -> Result<()> {
    if phases.is_empty() {
        return Ok(());
    }

    let mut ids: Vec<u8> = phases.iter().map(|p| p.id).collect();
    ids.sort_unstable();

    for (i, id) in ids.iter().enumerate() {
        let expected = (i as u8) + 1;
        if *id != expected {
            anyhow::bail!(
                "phase IDs must be contiguous starting from 1; expected {expected} but found {id}"
            );
        }
    }

    Ok(())
}

/// Ensure no two agents share the same key.
fn validate_unique_agent_keys(agents: &[ManifestAgent]) -> Result<()> {
    let mut seen = HashSet::new();
    for agent in agents {
        if !seen.insert(&agent.key) {
            anyhow::bail!("duplicate agent key: '{}'", agent.key);
        }
    }
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use tempfile::tempdir;

    /// Helper: build a valid TOML string with the given phases and agents.
    fn valid_toml() -> &'static str {
        r#"
[pipeline]
name = "coding"
version = "1.0"
description = "48-agent coding pipeline"
default_model = "sonnet"
context_window = 200000

[defaults]
algorithm = "ReAct"
model = "sonnet"

[[phases]]
id = 1
name = "Understanding"
tool_access = "ReadOnly"

[[phases]]
id = 2
name = "Design"
tool_access = "ReadOnly"

[[agent]]
key = "task-analyzer"
phase = 1
critical = true

[[agent]]
key = "requirement-extractor"
phase = 1
critical = false

[[agent]]
key = "architect"
phase = 2
critical = true
"#
    }

    #[test]
    fn test_load_manifest_valid() {
        let manifest = parse_manifest(valid_toml()).expect("should parse valid TOML");

        // Pipeline meta
        assert_eq!(manifest.pipeline.name, "coding");
        assert_eq!(manifest.pipeline.version, "1.0");
        assert_eq!(
            manifest.pipeline.description.as_deref(),
            Some("48-agent coding pipeline")
        );
        assert_eq!(manifest.pipeline.default_model.as_deref(), Some("sonnet"));
        assert_eq!(manifest.pipeline.context_window, Some(200_000));

        // Defaults
        assert_eq!(manifest.defaults.algorithm.as_deref(), Some("ReAct"));
        assert_eq!(manifest.defaults.model.as_deref(), Some("sonnet"));

        // Phases
        assert_eq!(manifest.phases.len(), 2);
        assert_eq!(manifest.phases[0].id, 1);
        assert_eq!(manifest.phases[0].name, "Understanding");
        assert_eq!(manifest.phases[1].id, 2);
        assert_eq!(manifest.phases[1].name, "Design");

        // Agents
        assert_eq!(manifest.agents.len(), 3);
        assert_eq!(manifest.agents[0].key, "task-analyzer");
        assert!(manifest.agents[0].critical);
        assert_eq!(manifest.agents[1].key, "requirement-extractor");
        assert!(!manifest.agents[1].critical);
        assert_eq!(manifest.agents[2].key, "architect");
        assert!(manifest.agents[2].critical);
    }

    #[test]
    fn test_load_manifest_duplicate_agent_key() {
        let toml = r#"
[pipeline]
name = "test"
version = "1.0"

[[phases]]
id = 1
name = "Phase1"

[[agent]]
key = "analyzer"
phase = 1

[[agent]]
key = "analyzer"
phase = 1
"#;
        let err = parse_manifest(toml).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("duplicate agent key"),
            "expected duplicate key error, got: {msg}"
        );
    }

    #[test]
    fn test_load_manifest_non_contiguous_phases() {
        let toml = r#"
[pipeline]
name = "test"
version = "1.0"

[[phases]]
id = 1
name = "Phase1"

[[phases]]
id = 3
name = "Phase3"

[[agent]]
key = "a"
phase = 1
"#;
        let err = parse_manifest(toml).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("contiguous"),
            "expected contiguous error, got: {msg}"
        );
    }

    #[test]
    fn test_load_manifest_defaults_optional() {
        let toml = r#"
[pipeline]
name = "minimal"
version = "0.1"

[[phases]]
id = 1
name = "Only"

[[agent]]
key = "solo"
phase = 1
"#;
        let manifest = parse_manifest(toml).expect("should parse without [defaults]");
        assert!(manifest.defaults.algorithm.is_none());
        assert!(manifest.defaults.model.is_none());
    }

    #[test]
    fn test_validate_missing_md_file() {
        let manifest = parse_manifest(valid_toml()).unwrap();
        // agent_keys is empty — every manifest agent is "missing"
        let warnings = validate(&manifest, &[]);
        assert_eq!(warnings.len(), 3);
        assert!(warnings[0].contains("no matching agent file"));
    }

    #[test]
    fn test_validate_unreferenced_md_file() {
        let manifest = parse_manifest(valid_toml()).unwrap();
        let keys = vec![
            "task-analyzer".to_string(),
            "requirement-extractor".to_string(),
            "architect".to_string(),
            "orphan-agent".to_string(),
        ];
        let warnings = validate(&manifest, &keys);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("orphan-agent"));
        assert!(warnings[0].contains("not referenced"));
    }

    #[test]
    fn test_validate_all_matched() {
        let manifest = parse_manifest(valid_toml()).unwrap();
        let keys = vec![
            "task-analyzer".to_string(),
            "requirement-extractor".to_string(),
            "architect".to_string(),
        ];
        let warnings = validate(&manifest, &keys);
        assert!(warnings.is_empty(), "expected no warnings: {warnings:?}");
    }

    #[test]
    fn test_load_manifest_nonexistent_file() {
        let result = load_manifest(Path::new("/tmp/does-not-exist-manifest.toml"));
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("failed to read manifest file"));
    }

    #[test]
    fn test_manifest_agents_sorted_by_phase_order() {
        let toml = r#"
[pipeline]
name = "order-test"
version = "1.0"

[[phases]]
id = 1
name = "First"

[[phases]]
id = 2
name = "Second"

[[agent]]
key = "beta"
phase = 2

[[agent]]
key = "alpha"
phase = 1

[[agent]]
key = "gamma"
phase = 2
"#;
        let manifest = parse_manifest(toml).unwrap();
        // Agents should preserve declaration order from the TOML, NOT be sorted.
        assert_eq!(manifest.agents[0].key, "beta");
        assert_eq!(manifest.agents[1].key, "alpha");
        assert_eq!(manifest.agents[2].key, "gamma");
    }

    #[test]
    fn test_load_manifest_from_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("pipeline.toml");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(valid_toml().as_bytes()).unwrap();

        let manifest = load_manifest(&path).expect("should load from file");
        assert_eq!(manifest.pipeline.name, "coding");
        assert_eq!(manifest.agents.len(), 3);
    }

    #[test]
    fn test_manifest_critical_flag_defaults_false() {
        let toml = r#"
[pipeline]
name = "critical-default-test"
version = "1.0"

[[phases]]
id = 1
name = "Phase1"

[[agent]]
key = "no-critical-field"
phase = 1
"#;
        let manifest = parse_manifest(toml).expect("should parse");
        assert_eq!(manifest.agents.len(), 1);
        assert!(
            !manifest.agents[0].critical,
            "critical should default to false when omitted"
        );
    }

    #[test]
    fn test_manifest_missing_phases_is_error() {
        // The `phases` field is required (no `#[serde(default)]`), so omitting
        // it entirely should produce a parse error.
        let toml = r#"
[pipeline]
name = "no-phases"
version = "1.0"

[[agent]]
key = "lonely"
phase = 1
"#;
        let err = parse_manifest(toml).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("missing field") || msg.contains("phases"),
            "expected missing-field error for phases, got: {msg}"
        );
    }

    #[test]
    fn test_manifest_single_phase_validates() {
        // A manifest with exactly one phase (id=1) should pass validation.
        let toml = r#"
[pipeline]
name = "single-phase"
version = "1.0"

[[phases]]
id = 1
name = "OnlyPhase"

[[agent]]
key = "solo"
phase = 1
"#;
        let manifest = parse_manifest(toml).expect("should parse single-phase manifest");
        assert_eq!(manifest.phases.len(), 1);
        assert_eq!(manifest.phases[0].name, "OnlyPhase");
    }

    #[test]
    fn test_manifest_agent_optional_fields() {
        // ManifestAgent only has key/phase/critical, but PipelineMeta and
        // PhaseDefinition carry the optional fields (model via defaults,
        // context_budget via context_window, tool_access on phases).
        // Verify these optional fields are captured when present.
        let toml = r#"
[pipeline]
name = "optional-fields-test"
version = "2.0"
description = "Testing optional fields"
default_model = "opus"
context_window = 150000

[defaults]
algorithm = "LATS"
model = "haiku"

[[phases]]
id = 1
name = "Analysis"
tool_access = "ReadOnly"

[[phases]]
id = 2
name = "Implementation"
tool_access = "Full"

[[agent]]
key = "analyzer"
phase = 1
critical = true
"#;
        let manifest = parse_manifest(toml).expect("should parse");

        // Pipeline optional fields
        assert_eq!(manifest.pipeline.default_model.as_deref(), Some("opus"));
        assert_eq!(manifest.pipeline.context_window, Some(150_000));
        assert_eq!(
            manifest.pipeline.description.as_deref(),
            Some("Testing optional fields")
        );

        // Defaults optional fields
        assert_eq!(manifest.defaults.algorithm.as_deref(), Some("LATS"));
        assert_eq!(manifest.defaults.model.as_deref(), Some("haiku"));

        // Phase tool_access optional field
        assert_eq!(manifest.phases[0].tool_access.as_deref(), Some("ReadOnly"));
        assert_eq!(manifest.phases[1].tool_access.as_deref(), Some("Full"));

        // Agent
        assert_eq!(manifest.agents[0].key, "analyzer");
        assert!(manifest.agents[0].critical);
    }
}
