//! The shapes the knowledge-base API exchanges with callers.
//!
//! Owns the request and result types for the `kb_nodes` operations — ingest,
//! lint — plus the aggregate statistics. Pure data: nothing here reads a
//! database or touches the filesystem, which is what lets the operation modules
//! and their callers share a vocabulary without sharing a dependency.
//!
//! Compile and query no longer appear here: they returned `kb_nodes` shapes,
//! and both now work on documents, so they own their own result types in
//! [`super::compile`] and [`super::query`].

use serde::{Deserialize, Serialize};

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
