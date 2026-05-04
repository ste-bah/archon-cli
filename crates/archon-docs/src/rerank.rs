//! Optional local reranker abstraction per TSPEC-ARCHON-EVIDENCE-ENGINE-001 §6.3.
//!
//! The reranker re-scores candidate passages after initial retrieval. It is
//! optional: if no reranker is configured, search results are returned in
//! embedding-similarity order without rescoring.

use crate::errors::DocsError;

/// A relevance score assigned to a single passage.
#[derive(Clone, Debug)]
pub struct RerankScore {
    /// Index into the original passages slice.
    pub index: usize,
    /// Relevance score (higher = more relevant).
    pub relevance_score: f32,
}

/// Local cross-encoder reranker.
pub trait LocalReranker: Send + Sync {
    /// Re-score passages against a query. Returns scores sorted DESC by
    /// relevance_score. `passages` is the list of candidate texts.
    fn rerank(&self, query: &str, passages: &[String]) -> Result<Vec<RerankScore>, DocsError>;

    /// Human-readable backend name.
    fn backend_name(&self) -> &'static str;
}

// ---------------------------------------------------------------------------
// No-op reranker (pass-through)
// ---------------------------------------------------------------------------

/// A reranker that returns identity scores (no rescoring).
pub struct NoOpReranker;

impl LocalReranker for NoOpReranker {
    fn rerank(&self, _query: &str, passages: &[String]) -> Result<Vec<RerankScore>, DocsError> {
        Ok(passages
            .iter()
            .enumerate()
            .map(|(i, _)| RerankScore {
                index: i,
                relevance_score: 1.0,
            })
            .collect())
    }

    fn backend_name(&self) -> &'static str {
        "noop"
    }
}
