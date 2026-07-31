use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;

use dashmap::DashMap;

use crate::agents::metadata::AgentMetadata;
use crate::agents::schema::ValidationReport;

/// Logic for combining filter criteria.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FilterLogic {
    #[default]
    And,
    Or,
}

/// Filter criteria for listing agents from the catalog.
#[derive(Debug, Clone, Default)]
pub struct AgentFilter {
    pub tags: Vec<String>,
    pub capabilities: Vec<String>,
    pub name_pattern: Option<globset::Glob>,
    pub version_req: Option<semver::VersionReq>,
    pub logic: FilterLogic,
    pub include_invalid: bool,
}

/// Detailed view of a single agent returned by `catalog.info()`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentInfoView {
    pub selected: AgentMetadata,
    pub all_versions: Vec<semver::Version>,
    pub dependency_graph: Vec<AgentKey>,
}

/// Composite key for a catalog entry: (name, version).
pub type AgentKey = (String, semver::Version);

/// Atomic snapshot of the entire catalog — torn-read-free via ArcSwap.
#[derive(Debug, Default)]
pub struct CatalogSnapshot {
    pub entries: DashMap<AgentKey, AgentMetadata>,
    pub name_index: DashMap<String, BTreeSet<semver::Version>>,
    pub tag_index: DashMap<String, HashSet<AgentKey>>,
    pub capability_index: DashMap<String, HashSet<AgentKey>>,
}

impl Clone for CatalogSnapshot {
    fn clone(&self) -> Self {
        let new = Self::default();
        for entry in self.entries.iter() {
            new.entries
                .insert(entry.key().clone(), entry.value().clone());
        }
        for entry in self.name_index.iter() {
            new.name_index
                .insert(entry.key().clone(), entry.value().clone());
        }
        for entry in self.tag_index.iter() {
            new.tag_index
                .insert(entry.key().clone(), entry.value().clone());
        }
        for entry in self.capability_index.iter() {
            new.capability_index
                .insert(entry.key().clone(), entry.value().clone());
        }
        new
    }
}

/// Structured evidence for a dependency that cannot be resolved.
#[derive(Debug, thiserror::Error)]
#[error("unresolved dependency: {required_by:?} requires {name} ({version_req})")]
pub struct UnresolvedDependency {
    pub required_by: AgentKey,
    pub name: String,
    pub version_req: semver::VersionReq,
    pub suggestions: Vec<String>,
}

/// Errors from the discovery subsystem.
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("schema validation failed: {0}")]
    Schema(String),

    #[error("metadata too large: {path:?} ({size} bytes, max 10 MB)")]
    MetadataTooLarge { path: PathBuf, size: usize },

    #[error("circular dependency: {0:?}")]
    CircularDependency(Vec<String>),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("{0}")]
    UnresolvedDependency(Box<UnresolvedDependency>),

    #[error("agent not found: {name} (did you mean: {suggestions:?})")]
    AgentNotFound {
        name: String,
        suggestions: Vec<String>,
    },
}

impl From<ValidationReport> for DiscoveryError {
    fn from(report: ValidationReport) -> Self {
        Self::Schema(report.reason())
    }
}

/// Configuration for a discovery source.
pub struct DiscoverySourceConfig {
    pub kind: DiscoverySourceKind,
    pub priority: u8,
}

/// The kind of discovery source.
pub enum DiscoverySourceKind {
    LocalDir(PathBuf),
    RemoteHttp { url: String, ttl_secs: u64 },
}
