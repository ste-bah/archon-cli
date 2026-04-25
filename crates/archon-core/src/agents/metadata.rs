// TASK-AGS-300: Canonical AgentMetadata types for the agent discovery system.
//
// These types represent discovered agent metadata (name, version, tags,
// capabilities, schemas, resource requirements) as opposed to the runtime
// agent definitions in `definition.rs` / `CustomAgentDefinition`.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Metadata for a discovered agent entry in the catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetadata {
    pub name: String,
    pub version: semver::Version,
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default = "default_schema")]
    pub input_schema: serde_json::Value,
    #[serde(default = "default_schema")]
    pub output_schema: serde_json::Value,
    #[serde(default)]
    pub resource_requirements: ResourceReq,
    #[serde(default)]
    pub dependencies: Vec<DependencyRef>,
    #[serde(default)]
    pub source_path: PathBuf,
    #[serde(default)]
    pub source_kind: SourceKind,
    #[serde(default)]
    pub state: AgentState,
    #[serde(default = "Utc::now")]
    pub loaded_at: DateTime<Utc>,
}

fn default_schema() -> serde_json::Value {
    serde_json::json!({})
}

/// Resource requirements for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceReq {
    #[serde(default = "default_cpu")]
    pub cpu: f32,
    #[serde(default)]
    pub memory_mb: u64,
    #[serde(default = "default_timeout")]
    pub timeout_sec: u64,
}

fn default_cpu() -> f32 {
    1.0
}
fn default_timeout() -> u64 {
    300
}

impl Default for ResourceReq {
    fn default() -> Self {
        Self {
            cpu: default_cpu(),
            memory_mb: 0,
            timeout_sec: default_timeout(),
        }
    }
}

/// A dependency on another agent by name and version requirement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyRef {
    pub name: String,
    #[serde(default = "default_version_req")]
    pub version_req: semver::VersionReq,
}

fn default_version_req() -> semver::VersionReq {
    semver::VersionReq::STAR
}

/// Where the agent metadata was loaded from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    #[default]
    Local,
    Remote,
}

/// Validation/staleness state of a discovered agent entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AgentState {
    #[default]
    Valid,
    Invalid(String),
    Stale,
}
