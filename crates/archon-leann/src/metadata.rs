//! File and symbol metadata.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Metadata about a chunk of source code within a file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeMetadata {
    pub file_path: PathBuf,
    pub language: String,
    pub line_start: usize,
    pub line_end: usize,
    pub chunk_content: String,
    pub file_hash: String,
}

/// A code chunk with its computed embedding vector.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeChunk {
    pub metadata: CodeMetadata,
    pub embedding: Vec<f32>,
}

/// A single search result returned by the code index.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub file_path: PathBuf,
    pub content: String,
    pub language: String,
    pub line_start: usize,
    pub line_end: usize,
    pub relevance_score: f64,
}

/// Configuration for indexing a repository.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexConfig {
    pub root_path: PathBuf,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
}

/// Statistics about the current index state.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IndexStats {
    pub total_files: usize,
    pub total_chunks: usize,
    pub index_size_bytes: u64,
    pub languages: HashMap<String, usize>,
    pub created_at: Option<String>,
}

/// Result of processing the indexing queue.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueueResult {
    pub processed: usize,
    pub failed: usize,
    pub remaining: usize,
}
