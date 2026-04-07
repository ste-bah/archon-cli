//! Evidence pack types and validation for the repo-evidence agent.
//!
//! An `EvidencePack` captures file:line evidence for every claim made
//! during codebase analysis, along with call graphs, test references,
//! entrypoints, and API contracts.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A complete evidence pack produced by the context-gatherer agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidencePack {
    pub facts: Vec<EvidenceFact>,
    pub call_graph: CallGraph,
    pub existing_tests: Vec<TestReference>,
    pub entrypoints: Vec<Entrypoint>,
    pub api_contracts: Vec<ApiContract>,
}

/// A single evidence-backed fact with a file:line reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceFact {
    pub claim: String,
    pub evidence: FileLineRef,
    pub tool_used: String,
    pub verified_at: String,
}

/// A reference to a specific file and line number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLineRef {
    pub file: String,
    pub line: u32,
}

/// A directed call graph consisting of caller→callee edges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallGraph {
    pub edges: Vec<CallEdge>,
}

/// A single edge in the call graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEdge {
    pub caller: FileLineRef,
    pub callee: FileLineRef,
    pub function_name: String,
}

/// A reference to an existing test in the codebase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestReference {
    pub test_file: String,
    pub test_function: String,
    pub covers_module: String,
}

/// An entrypoint into the application (binary main, HTTP handler, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entrypoint {
    pub file: String,
    pub function: String,
    pub entrypoint_type: String,
}

/// An API contract describing an endpoint's request/response types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiContract {
    pub endpoint: String,
    pub method: String,
    pub request_type: Option<String>,
    pub response_type: Option<String>,
    pub file: String,
    pub line: u32,
}

/// An error indicating an unsourced or invalid claim in an evidence pack.
#[derive(Debug, Clone)]
pub struct EvidenceValidationError {
    pub claim: String,
    pub reason: String,
}

impl std::fmt::Display for EvidenceValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Unsourced claim '{}': {}", self.claim, self.reason)
    }
}

impl std::error::Error for EvidenceValidationError {}

/// Validate that every fact in the evidence pack has a non-empty file
/// and a line number >= 1. Returns all validation errors if any are found.
pub fn validate_evidence_pack(pack: &EvidencePack) -> std::result::Result<(), Vec<EvidenceValidationError>> {
    let mut errors = Vec::new();
    for fact in &pack.facts {
        if fact.evidence.file.trim().is_empty() {
            errors.push(EvidenceValidationError {
                claim: fact.claim.clone(),
                reason: "evidence.file is empty".into(),
            });
        }
        if fact.evidence.line == 0 {
            errors.push(EvidenceValidationError {
                claim: fact.claim.clone(),
                reason: "evidence.line is 0 (must be >= 1)".into(),
            });
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Save an evidence pack as `evidence.json` in the given session directory.
pub fn save_evidence_pack(pack: &EvidencePack, session_dir: &Path) -> Result<()> {
    let path = session_dir.join("evidence.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(pack)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Load an evidence pack from `evidence.json` in the given session directory.
pub fn load_evidence_pack(session_dir: &Path) -> Result<EvidencePack> {
    let path = session_dir.join("evidence.json");
    let data = std::fs::read_to_string(&path)?;
    let pack: EvidencePack = serde_json::from_str(&data)?;
    Ok(pack)
}
