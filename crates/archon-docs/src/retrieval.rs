//! Document retrieval for chunks.
//!
//! Provides the search pipeline: exact/FTS retrieval, semantic Rust-HNSW
//! retrieval over RocksDB raw vectors, and hybrid score fusion.

use std::collections::HashMap;

use cozo::DbInstance;

use crate::errors::DocsError;
use crate::models::ChunkArtifact;
use crate::retrieval_exact::{
    ExactFallback, exact_search, expanded_top_k, relaxed_exact_search, score_exact_for_candidates,
};
use crate::retrieval_query::safe_fts_query;
use crate::retrieval_semantic::semantic_search_with_norm;
use crate::store;

const HYBRID_EXACT_SHORTCUT_MIN_RESULTS: usize = 3;
const HYBRID_EXACT_SHORTCUT_SCORE: f64 = 0.75;
const HYBRID_EXACT_SHORTCUT_MIN_TERMS: usize = 2;

/// A single search result with chunk content and relevance metadata.
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub chunk_id: String,
    pub document_id: String,
    pub content: String,
    pub page_start: u32,
    pub page_end: u32,
    /// Cosine distance from the query (0 = identical, 2 = opposite).
    pub distance: f64,
    /// Relevance score (1.0 - distance/2, so 1.0 = perfect match).
    pub score: f64,
    /// Exact/FTS contribution after normalisation.
    pub exact_score: f64,
    /// Semantic HNSW contribution after normalisation.
    pub semantic_score: f64,
}

/// Batch of search results returned from a query.
#[derive(Clone, Debug)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
    pub query: String,
    pub query_embedding_norm: Option<f64>,
    pub total_chunks: usize,
    pub total_indexed_chunks: usize,
    pub mode: SearchMode,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchMode {
    Exact,
    Semantic,
    Hybrid,
}

impl SearchMode {
    pub fn parse(value: &str) -> Result<Self, DocsError> {
        match value {
            "exact" => Ok(Self::Exact),
            "semantic" => Ok(Self::Semantic),
            "hybrid" => Ok(Self::Hybrid),
            other => Err(DocsError::Validation {
                message: format!("unsupported docs search mode '{other}'"),
            }),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Semantic => "semantic",
            Self::Hybrid => "hybrid",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RetrievalWeights {
    pub exact: f64,
    pub semantic: f64,
}

impl Default for RetrievalWeights {
    fn default() -> Self {
        Self {
            exact: 0.45,
            semantic: 0.55,
        }
    }
}

impl RetrievalWeights {
    fn normalized(self) -> Self {
        let exact = self.exact.max(0.0);
        let semantic = self.semantic.max(0.0);
        let total = exact + semantic;
        if total <= f64::EPSILON {
            return Self::default();
        }
        Self {
            exact: exact / total,
            semantic: semantic / total,
        }
    }
}

/// Search for chunks similar to `query`.
pub fn search(db: &DbInstance, query: &str, top_k: usize) -> Result<SearchResults, DocsError> {
    search_with_mode(
        db,
        query,
        top_k,
        SearchMode::Hybrid,
        RetrievalWeights::default(),
    )
}

pub fn search_with_policy(
    db: &DbInstance,
    query: &str,
    top_k: usize,
    mode: SearchMode,
    policy: &archon_policy::EffectivePolicy,
) -> Result<SearchResults, DocsError> {
    let weights = RetrievalWeights {
        exact: policy.docs.retrieval.exact_weight,
        semantic: policy.docs.retrieval.semantic_weight,
    };
    search_with_mode(db, query, top_k, mode, weights)
}

pub fn search_with_mode(
    db: &DbInstance,
    query: &str,
    top_k: usize,
    mode: SearchMode,
    weights: RetrievalWeights,
) -> Result<SearchResults, DocsError> {
    let total = count_indexed(db)?;
    let total_chunks = count_chunks(db)?;
    if matches!(mode, SearchMode::Semantic) && total == 0 {
        return empty_or_failed_result(db, query, mode, total_chunks);
    }

    let mut warnings = Vec::new();
    let mut query_embedding_norm = None;
    let results = match mode {
        SearchMode::Exact => exact_search(db, query, top_k, total_chunks, ExactFallback::Strict)?,
        SearchMode::Semantic => {
            let semantic = semantic_search_with_norm(db, query, top_k)?;
            query_embedding_norm = Some(semantic.query_embedding_norm);
            semantic.results
        }
        SearchMode::Hybrid => hybrid_search(
            db,
            query,
            top_k,
            weights,
            total,
            total_chunks,
            &mut warnings,
            &mut query_embedding_norm,
        )?,
    };
    if !matches!(mode, SearchMode::Exact) && total == 0 && results.is_empty() {
        fail_if_indexing_failed(db)?;
    }

    Ok(SearchResults {
        results,
        query: query.to_string(),
        query_embedding_norm,
        total_chunks,
        total_indexed_chunks: total,
        mode,
        warnings,
    })
}

fn count_indexed(db: &DbInstance) -> Result<usize, DocsError> {
    store::count_indexed_chunks(db).map_err(|e| DocsError::Retrieval {
        message: e.to_string(),
    })
}

fn count_chunks(db: &DbInstance) -> Result<usize, DocsError> {
    store::count_chunks(db).map_err(|e| DocsError::Retrieval {
        message: e.to_string(),
    })
}

fn empty_or_failed_result(
    db: &DbInstance,
    query: &str,
    mode: SearchMode,
    total_chunks: usize,
) -> Result<SearchResults, DocsError> {
    fail_if_indexing_failed(db)?;
    Ok(SearchResults {
        results: Vec::new(),
        query: query.to_string(),
        query_embedding_norm: None,
        total_chunks,
        total_indexed_chunks: 0,
        mode,
        warnings: Vec::new(),
    })
}

fn fail_if_indexing_failed(db: &DbInstance) -> Result<(), DocsError> {
    let failed_count = store::count_failed_chunks(db).map_err(|e| DocsError::Retrieval {
        message: e.to_string(),
    })?;
    if failed_count == 0 {
        return Ok(());
    }
    Err(DocsError::Embedding {
        message: format!(
            "{failed_count} chunks failed indexing. Run 'archon docs model-status' for diagnostics."
        ),
    })
}

fn hybrid_search(
    db: &DbInstance,
    query: &str,
    top_k: usize,
    weights: RetrievalWeights,
    total_indexed_chunks: usize,
    total_chunks: usize,
    warnings: &mut Vec<String>,
    query_embedding_norm: &mut Option<f64>,
) -> Result<Vec<SearchResult>, DocsError> {
    if top_k == 0 {
        return Ok(Vec::new());
    }
    let mut exact = exact_search(
        db,
        query,
        expanded_top_k(top_k),
        total_chunks,
        ExactFallback::BestEffort,
    )?;
    if exact.is_empty() && is_definition_query(query) {
        exact = relaxed_exact_search(db, query, expanded_top_k(top_k))?;
    }
    if should_use_exact_only(query, &exact, top_k) {
        warnings.push(
            "semantic retrieval skipped: high-confidence lexical evidence was sufficient".into(),
        );
        return Ok(exact.into_iter().take(top_k).collect());
    }

    let semantic = semantic_or_warning(db, query, top_k, total_indexed_chunks, warnings)?;
    if let Some(norm) = semantic.as_ref().map(|search| search.query_embedding_norm) {
        *query_embedding_norm = Some(norm);
    }
    Ok(fuse_results(
        query,
        exact,
        semantic.map(|search| search.results).unwrap_or_default(),
        weights,
        top_k,
    ))
}

fn should_use_exact_only(query: &str, results: &[SearchResult], top_k: usize) -> bool {
    if std::env::var_os("ARCHON_DOCS_HYBRID_ALWAYS_SEMANTIC").is_some() {
        return false;
    }
    if results.is_empty() {
        return false;
    }
    if has_specific_exact_hit(query, results) {
        return true;
    }
    let required = top_k.min(HYBRID_EXACT_SHORTCUT_MIN_RESULTS);
    required > 0
        && results.len() >= required
        && results
            .iter()
            .take(required)
            .all(|result| result.exact_score >= HYBRID_EXACT_SHORTCUT_SCORE)
}

fn has_specific_exact_hit(query: &str, results: &[SearchResult]) -> bool {
    let terms = shortcut_terms(query);
    let definition_query = is_definition_query(query);
    if terms.len() < 3 && !(definition_query && terms.len() >= HYBRID_EXACT_SHORTCUT_MIN_TERMS) {
        return false;
    }
    let threshold = required_term_matches(definition_query, terms.len());
    results
        .iter()
        .take(HYBRID_EXACT_SHORTCUT_MIN_RESULTS)
        .any(|result| matched_term_count(&result.content, &terms) >= threshold)
}

fn shortcut_terms(query: &str) -> Vec<String> {
    safe_fts_query(query)
        .split_whitespace()
        .filter(|term| term.len() > 1)
        .take(8)
        .map(ToString::to_string)
        .collect()
}

fn required_term_matches(definition_query: bool, term_count: usize) -> usize {
    if definition_query && term_count == HYBRID_EXACT_SHORTCUT_MIN_TERMS {
        return 1;
    }
    term_count.min(4)
}

fn is_definition_query(query: &str) -> bool {
    let lower = query.to_lowercase();
    lower.contains("what is")
        || lower.contains("what do you know")
        || lower.contains("tell me about")
        || lower.contains("explain")
}

fn matched_term_count(content: &str, terms: &[String]) -> usize {
    let lower = content.to_lowercase();
    terms
        .iter()
        .filter(|term| lower.contains(term.as_str()))
        .count()
}

fn semantic_or_warning(
    db: &DbInstance,
    query: &str,
    top_k: usize,
    total_indexed_chunks: usize,
    warnings: &mut Vec<String>,
) -> Result<Option<crate::retrieval_semantic::SemanticSearch>, DocsError> {
    if total_indexed_chunks == 0 {
        warnings.push("semantic retrieval skipped: no indexed vectors".into());
        return Ok(None);
    }
    match semantic_search_with_norm(db, query, top_k.saturating_mul(3).max(top_k)) {
        Ok(semantic) => Ok(Some(semantic)),
        Err(DocsError::ModelNotConfigured { message }) => {
            warnings.push(format!(
                "semantic retrieval skipped: no embedding provider configured ({message})"
            ));
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

fn fuse_results(
    query: &str,
    exact: Vec<SearchResult>,
    semantic: Vec<SearchResult>,
    weights: RetrievalWeights,
    top_k: usize,
) -> Vec<SearchResult> {
    let mut merged: HashMap<String, SearchResult> = exact
        .into_iter()
        .map(|result| (result.chunk_id.clone(), result))
        .collect();
    for semantic_result in semantic {
        merged
            .entry(semantic_result.chunk_id.clone())
            .and_modify(|existing| {
                existing.semantic_score = semantic_result.semantic_score;
                existing.distance = semantic_result.distance;
            })
            .or_insert(semantic_result);
    }

    score_exact_for_candidates(query, &mut merged);
    let weights = weights.normalized();
    let mut results: Vec<SearchResult> = merged
        .into_values()
        .map(|mut result| {
            result.score =
                weights.exact * result.exact_score + weights.semantic * result.semantic_score;
            result
        })
        .collect();
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(top_k);
    results
}

pub use crate::indexing::IndexResult;

pub fn index_chunk(db: &DbInstance, chunk: &ChunkArtifact) -> Result<(), DocsError> {
    crate::indexing::index_chunk(db, chunk)
}

/// Index all chunks with embedding_status = "pending".
pub fn index_pending_chunks(db: &DbInstance) -> Result<IndexResult, DocsError> {
    crate::indexing::index_chunks(db, &crate::indexing::IndexOptions::default())
}

/// Re-index all chunks regardless of status (useful after model swap).
pub fn reindex_all(db: &DbInstance) -> Result<IndexResult, DocsError> {
    crate::indexing::index_chunks(
        db,
        &crate::indexing::IndexOptions {
            all: true,
            ..Default::default()
        },
    )
}
