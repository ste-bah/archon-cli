//! Data shapes for the Q&A engine.
//!
//! Split out of [`super`] so the engine file stays inside the repo's 500-line
//! ceiling once streaming was added; these are inert structs with no behaviour,
//! and `query` re-exports every one of them, so `kb::query::QaQueryResult` and
//! friends keep resolving exactly as before.

use archon_docs::retrieval::SearchMode;
use serde::{Deserialize, Serialize};

/// Options for a Q&A query.
#[derive(Clone, Debug)]
pub struct QaQueryOptions {
    pub top_k: usize,
    /// Store the answer as a searchable document.
    pub file_answer: bool,
    /// Include summaries and concept articles derived from the cited documents.
    pub include_derived_context: bool,
    pub mode: SearchMode,
    /// Restrict retrieval to one named knowledge base.
    pub kb: Option<String>,
}

impl Default for QaQueryOptions {
    fn default() -> Self {
        Self {
            top_k: 10,
            file_answer: false,
            include_derived_context: true,
            mode: SearchMode::Hybrid,
            kb: None,
        }
    }
}

/// A scored chunk from retrieval.
#[derive(Clone, Debug)]
pub struct ScoredChunk {
    pub chunk_id: String,
    pub document_id: String,
    pub source_path: String,
    pub content: String,
    pub score: f64,
}

/// Context assembled around the retrieved chunks.
#[derive(Clone, Debug, Default)]
pub struct AnswerContext {
    pub primary: Vec<ScoredChunk>,
    /// Compiled summaries of the documents the primary chunks came from.
    pub summaries: Vec<String>,
    /// Concept articles linked to those documents.
    pub concepts: Vec<String>,
}

/// A synthesized answer with source citations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SynthesizedAnswer {
    pub answer_text: String,
    pub source_citations: Vec<SourceCitation>,
}

/// Citation referencing a retrieved chunk.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceCitation {
    pub chunk_id: String,
    pub document_id: String,
    pub quote: String,
    pub relevance: f64,
}

/// Full result of a Q&A query.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QaQueryResult {
    pub answer: String,
    pub sources: Vec<QaSource>,
    /// Document ID the answer was filed as, when `file_answer` was set.
    pub filed_document_id: Option<String>,
    pub search_duration_ms: u64,
    pub synthesis_duration_ms: u64,
    /// Non-fatal notes from retrieval (e.g. "no embedding provider").
    pub warnings: Vec<String>,
}

/// Source info in a query result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QaSource {
    pub chunk_id: String,
    pub document_id: String,
    pub source_path: String,
    pub relevance_score: f64,
    pub quote: String,
}
