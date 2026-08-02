//! The shapes the knowledge-base API exchanges with callers.
//!
//! Owns the request and result types for the four knowledge-base operations —
//! ingest, compile, query, lint — plus the aggregate statistics. Pure data:
//! nothing here reads a database or touches the filesystem, which is what lets
//! the operation modules and their callers share a vocabulary without sharing
//! a dependency.

use serde::{Deserialize, Serialize};

use super::KbNode;

/// Source of content to ingest into the knowledge base.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum IngestSource {
    FilePath(std::path::PathBuf),
    Url(String),
    Directory(std::path::PathBuf),
}

/// Result of an ingest operation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IngestResult {
    pub nodes_created: usize,
    pub chunks_processed: usize,
    pub errors: Vec<String>,
}

/// Result of a compile (synthesis) pass over ingested content.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompileResult {
    pub articles_created: usize,
    pub concepts_extracted: usize,
}

/// Options for querying the knowledge base.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryOptions {
    pub max_results: usize,
    pub min_relevance: f64,
    pub domain_filter: Option<String>,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            max_results: 10,
            min_relevance: 0.0,
            domain_filter: None,
        }
    }
}

/// Result of a knowledge base query.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueryResult {
    pub answer: String,
    pub sources: Vec<KbNode>,
    pub confidence: f64,
}

/// Result of a lint pass over the knowledge base.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LintResult {
    pub issues_found: usize,
    pub suggestions: Vec<String>,
}

/// Aggregate statistics about the knowledge base.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct KbStats {
    pub total_nodes: usize,
    pub total_edges: usize,
    pub nodes_by_type: std::collections::HashMap<String, usize>,
}
