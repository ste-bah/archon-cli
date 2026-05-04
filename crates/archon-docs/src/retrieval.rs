//! CozoDB HNSW retrieval for document chunks.
//!
//! Provides the search pipeline: embed query → HNSW nearest-neighbour →
//! resolve chunks from doc_chunks → optional reranking → results with
//! provenance.

use std::collections::{BTreeMap, HashMap};

use cozo::{DataValue, DbInstance, ScriptMutability, Vector};

use crate::embed::get_provider;
use crate::errors::DocsError;
use crate::models::ChunkArtifact;
use crate::store;

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

/// Search for chunks similar to `query` using HNSW vector search.
///
/// If no embedding provider is configured, returns `DocsError::ModelNotConfigured`.
/// If no chunks are indexed, returns an empty result set.
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
    let total = store::count_embeddings(db).map_err(|e| DocsError::Retrieval {
        message: e.to_string(),
    })?;
    let total_chunks = store::count_chunks(db).map_err(|e| DocsError::Retrieval {
        message: e.to_string(),
    })?;

    if matches!(mode, SearchMode::Semantic) && total == 0 {
        let failed_count = store::count_failed_chunks(db).map_err(|e| DocsError::Retrieval {
            message: e.to_string(),
        })?;
        if failed_count > 0 {
            return Err(DocsError::Embedding {
                message: format!(
                    "{failed_count} chunks failed indexing. Run 'archon docs model-status' for diagnostics."
                ),
            });
        }
        return Ok(SearchResults {
            results: Vec::new(),
            query: query.to_string(),
            query_embedding_norm: None,
            total_chunks,
            total_indexed_chunks: 0,
            mode,
            warnings: Vec::new(),
        });
    }

    let mut warnings = Vec::new();
    let mut query_embedding_norm = None;
    let results = match mode {
        SearchMode::Exact => exact_search(db, query, top_k)?,
        SearchMode::Semantic => {
            let semantic = semantic_search_with_norm(db, query, top_k)?;
            query_embedding_norm = Some(semantic.query_embedding_norm);
            semantic.results
        }
        SearchMode::Hybrid => {
            let semantic_norm = &mut query_embedding_norm;
            hybrid_search(
                db,
                query,
                top_k,
                weights,
                total,
                &mut warnings,
                semantic_norm,
            )?
        }
    };
    if !matches!(mode, SearchMode::Exact) && total == 0 && results.is_empty() {
        let failed_count = store::count_failed_chunks(db).map_err(|e| DocsError::Retrieval {
            message: e.to_string(),
        })?;
        if failed_count > 0 {
            return Err(DocsError::Embedding {
                message: format!(
                    "{failed_count} chunks failed indexing. Run 'archon docs model-status' for diagnostics."
                ),
            });
        }
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

#[derive(Debug)]
struct SemanticSearch {
    results: Vec<SearchResult>,
    query_embedding_norm: f64,
}

fn semantic_search_with_norm(
    db: &DbInstance,
    query: &str,
    top_k: usize,
) -> Result<SemanticSearch, DocsError> {
    let provider = get_provider().ok_or_else(|| DocsError::ModelNotConfigured {
        message: "no embedding provider configured. Run 'archon docs model-status' for details."
            .into(),
    })?;
    let query_vec = provider.embed_query(query)?;
    let query_embedding_norm = l2_norm(&query_vec);
    let results = hnsw_search(db, &query_vec, top_k)?;
    Ok(SemanticSearch {
        results,
        query_embedding_norm,
    })
}

fn exact_search(
    db: &DbInstance,
    query: &str,
    top_k: usize,
) -> Result<Vec<SearchResult>, DocsError> {
    let needle = normalize_exact_query(query);
    if needle.is_empty() || top_k == 0 {
        return Ok(Vec::new());
    }

    let fts_results = match fts_search(db, &needle, top_k.saturating_mul(3).max(top_k)) {
        Ok(results) => results,
        Err(e) => {
            tracing::warn!(error = %e, "Cozo FTS search failed; falling back to exact scan");
            Vec::new()
        }
    };
    let scan_results = scan_exact_matches(db, &needle, top_k.saturating_mul(3).max(top_k))?;
    let mut merged: HashMap<String, SearchResult> = HashMap::new();
    for result in fts_results.into_iter().chain(scan_results) {
        merged
            .entry(result.chunk_id.clone())
            .and_modify(|existing| {
                if result.exact_score > existing.exact_score {
                    existing.exact_score = result.exact_score;
                    existing.score = result.score;
                    existing.distance = result.distance;
                }
            })
            .or_insert(result);
    }
    let mut results: Vec<SearchResult> = merged.into_values().collect();
    results.sort_by(|a, b| {
        b.exact_score
            .partial_cmp(&a.exact_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(top_k);
    Ok(results)
}

fn hybrid_search(
    db: &DbInstance,
    query: &str,
    top_k: usize,
    weights: RetrievalWeights,
    total_indexed_chunks: usize,
    warnings: &mut Vec<String>,
    query_embedding_norm: &mut Option<f64>,
) -> Result<Vec<SearchResult>, DocsError> {
    if top_k == 0 {
        return Ok(Vec::new());
    }
    let exact = exact_search(db, query, top_k.saturating_mul(3).max(top_k))?;
    let semantic = if total_indexed_chunks == 0 {
        warnings.push("semantic retrieval skipped: no indexed vectors".into());
        Vec::new()
    } else {
        match semantic_search_with_norm(db, query, top_k.saturating_mul(3).max(top_k)) {
            Ok(semantic) => {
                *query_embedding_norm = Some(semantic.query_embedding_norm);
                semantic.results
            }
            Err(DocsError::ModelNotConfigured { message }) => {
                warnings.push(format!(
                    "semantic retrieval skipped: no embedding provider configured ({message})"
                ));
                Vec::new()
            }
            Err(e) => return Err(e),
        }
    };

    let weights = weights.normalized();
    let mut merged: HashMap<String, SearchResult> = HashMap::new();
    for result in exact {
        merged.insert(result.chunk_id.clone(), result);
    }
    for semantic_result in semantic {
        merged
            .entry(semantic_result.chunk_id.clone())
            .and_modify(|existing| {
                existing.semantic_score = semantic_result.semantic_score;
                existing.distance = semantic_result.distance;
            })
            .or_insert(semantic_result);
    }

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
    Ok(results)
}

fn fts_search(db: &DbInstance, query: &str, top_k: usize) -> Result<Vec<SearchResult>, DocsError> {
    let mut params = BTreeMap::new();
    params.insert("query".to_string(), DataValue::from(query));
    params.insert("k".to_string(), DataValue::from(top_k as i64));

    let script = "?[score, chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] := \
        ~doc_chunks:chunk_content_fts {chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status | \
            query: $query, \
            k: $k, \
            score_kind: 'tf_idf', \
            bind_score: score \
        } \
        :order -score";

    let result = db
        .run_script(script, params, ScriptMutability::Immutable)
        .map_err(|e| DocsError::Retrieval {
            message: format!("FTS search failed: {e}"),
        })?;

    let max_score = result
        .rows
        .iter()
        .filter_map(|row| row[0].get_float())
        .fold(0.0_f64, f64::max);
    let denom = max_score.max(1e-9);

    Ok(result
        .rows
        .iter()
        .map(|row| {
            let raw_score = row[0].get_float().unwrap_or(0.0);
            let chunk = chunk_from_row(&row[1..]);
            exact_result_from_chunk(chunk, (raw_score / denom).clamp(0.0, 1.0))
        })
        .collect())
}

fn scan_exact_matches(
    db: &DbInstance,
    query: &str,
    top_k: usize,
) -> Result<Vec<SearchResult>, DocsError> {
    let query_lower = query.to_lowercase();
    let mut scored: Vec<(ChunkArtifact, f64)> = list_all_chunks(db)?
        .into_iter()
        .filter_map(|chunk| {
            let content_lower = chunk.content.to_lowercase();
            let count = content_lower.matches(&query_lower).count();
            let compact_query = query_lower.split_whitespace().collect::<Vec<_>>();
            let matched_terms = compact_query
                .iter()
                .filter(|term| content_lower.contains(**term))
                .count();
            if count == 0 && matched_terms == 0 {
                return None;
            }
            let term_score = if compact_query.is_empty() {
                0.0
            } else {
                matched_terms as f64 / compact_query.len() as f64
            };
            let occurrence_score = (count as f64 / 3.0).min(1.0);
            let score = if count == 0 {
                term_score * 0.7
            } else {
                (term_score + occurrence_score) / 2.0
            };
            Some((chunk, score.clamp(0.0, 1.0)))
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k);
    Ok(scored
        .into_iter()
        .map(|(chunk, score)| exact_result_from_chunk(chunk, score))
        .collect())
}

fn list_all_chunks(db: &DbInstance) -> Result<Vec<ChunkArtifact>, DocsError> {
    let result = db
        .run_script(
            "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] \
             := *doc_chunks{chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| DocsError::Retrieval {
            message: format!("list chunks for exact search failed: {e}"),
        })?;
    Ok(result.rows.iter().map(|row| chunk_from_row(row)).collect())
}

fn normalize_exact_query(query: &str) -> String {
    let trimmed = query.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

fn l2_norm(values: &[f32]) -> f64 {
    values
        .iter()
        .map(|value| {
            let value = f64::from(*value);
            value * value
        })
        .sum::<f64>()
        .sqrt()
}

fn exact_result_from_chunk(chunk: ChunkArtifact, score: f64) -> SearchResult {
    SearchResult {
        chunk_id: chunk.chunk_id,
        document_id: chunk.document_id,
        content: chunk.content,
        page_start: chunk.page_start,
        page_end: chunk.page_end,
        distance: 1.0 - score,
        score,
        exact_score: score,
        semantic_score: 0.0,
    }
}

fn chunk_from_row(row: &[DataValue]) -> ChunkArtifact {
    ChunkArtifact {
        chunk_id: row[0].get_str().unwrap_or("").to_string(),
        document_id: row[1].get_str().unwrap_or("").to_string(),
        artifact_id: row[2].get_str().unwrap_or("").to_string(),
        chunk_index: row[3].get_int().unwrap_or(0) as u32,
        page_start: row[4].get_int().unwrap_or(0) as u32,
        page_end: row[5].get_int().unwrap_or(0) as u32,
        content: row[6].get_str().unwrap_or("").to_string(),
        content_hash: row[7].get_str().unwrap_or("").to_string(),
        embedding_status: row[8].get_str().unwrap_or("pending").to_string(),
    }
}

/// Low-level HNSW search against vec_text_chunks.
fn hnsw_search(
    db: &DbInstance,
    query_vec: &[f32],
    top_k: usize,
) -> Result<Vec<SearchResult>, DocsError> {
    let arr = ndarray::Array1::from_vec(query_vec.to_vec());

    let mut params = BTreeMap::new();
    params.insert("query".to_string(), DataValue::Vec(Vector::F32(arr)));
    params.insert("k".to_string(), DataValue::from(top_k as i64));

    let script = "?[chunk_id, distance] := ~vec_text_chunks:chunk_embedding_idx{
            chunk_id,
            |
            query: $query,
            k: $k,
            ef: 50,
            bind_distance: distance
        }";

    let result = db
        .run_script(script, params, ScriptMutability::Immutable)
        .map_err(|e| DocsError::Retrieval {
            message: format!("HNSW search failed: {e}"),
        })?;

    let mut search_results = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        let chunk_id = row[0].get_str().unwrap_or("").to_string();
        let distance = row[1].get_float().unwrap_or(1.0);

        let chunk = match store::get_chunk_by_id(db, &chunk_id).map_err(|e| DocsError::Retrieval {
            message: e.to_string(),
        }) {
            Ok(Some(c)) => c,
            Ok(None) => {
                tracing::warn!(chunk_id = %chunk_id, "HNSW returned chunk_id not found in doc_chunks");
                continue;
            }
            Err(e) => {
                tracing::warn!(chunk_id = %chunk_id, error = %e, "failed to resolve chunk");
                continue;
            }
        };

        let semantic_score = 1.0 - distance / 2.0;
        search_results.push(SearchResult {
            score: semantic_score,
            distance,
            chunk_id,
            document_id: chunk.document_id,
            content: chunk.content,
            page_start: chunk.page_start,
            page_end: chunk.page_end,
            exact_score: 0.0,
            semantic_score,
        });
    }

    // HNSW returns approximate nearest neighbours; sort by distance ascending
    search_results.sort_by(|a, b| {
        a.distance
            .partial_cmp(&b.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(search_results)
}

/// Index a chunk: embed its content and store the vector.
/// On success, sets embedding_status to "indexed". On failure, sets it to "failed".
pub fn index_chunk(db: &DbInstance, chunk: &ChunkArtifact) -> Result<(), DocsError> {
    let provider = get_provider().ok_or_else(|| DocsError::ModelNotConfigured {
        message: "no embedding provider configured".into(),
    })?;

    crate::schema::ensure_vec_schema(db, provider.dimension()).map_err(|e| {
        DocsError::Retrieval {
            message: format!("failed to ensure vec schema: {e}"),
        }
    })?;

    match provider.embed_chunks(&[chunk.content.clone()]) {
        Ok(vectors) => {
            let embedding = vectors
                .into_iter()
                .next()
                .ok_or_else(|| DocsError::Embedding {
                    message: "embed_chunks returned empty".into(),
                })?;
            match store::insert_chunk_embedding(
                db,
                &chunk.chunk_id,
                &embedding,
                provider.backend_name(),
            ) {
                Ok(()) => {
                    let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "indexed");
                    Ok(())
                }
                Err(e) => {
                    let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
                    Err(DocsError::Retrieval {
                        message: e.to_string(),
                    })
                }
            }
        }
        Err(e) => {
            let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
            Err(e)
        }
    }
}

/// Result of an indexing operation with per-outcome counts.
#[derive(Clone, Debug, Default)]
pub struct IndexResult {
    /// Chunks successfully embedded and stored.
    pub indexed: usize,
    /// Chunks where embedding or storage failed.
    pub failed: usize,
    /// Chunks skipped because they were already indexed.
    pub skipped: usize,
}

/// Filter for selecting which chunks to index.
#[derive(Clone, Copy, Debug)]
enum ChunkFilter {
    /// Only chunks with embedding_status = "pending".
    OnlyPending,
    /// All chunks regardless of status.
    All,
}

/// Shared indexing loop: fetch chunks matching `filter`, embed, store, update status.
fn index_chunks_matching(db: &DbInstance, filter: ChunkFilter) -> Result<IndexResult, DocsError> {
    let provider = get_provider().ok_or_else(|| DocsError::ModelNotConfigured {
        message: "no embedding provider configured".into(),
    })?;

    crate::schema::ensure_vec_schema(db, provider.dimension()).map_err(|e| {
        DocsError::Retrieval {
            message: format!("failed to ensure vec schema: {e}"),
        }
    })?;

    let (script, error_label) = match filter {
        ChunkFilter::OnlyPending => (
            "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] \
             := *doc_chunks{chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status}, \
             embedding_status = \"pending\"",
            "list pending chunks failed",
        ),
        ChunkFilter::All => (
            "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] \
             := *doc_chunks{chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status}",
            "list all chunks failed",
        ),
    };

    let result = db
        .run_script(script, Default::default(), ScriptMutability::Immutable)
        .map_err(|e| DocsError::Retrieval {
            message: format!("{error_label}: {e}"),
        })?;

    let mut indexed = 0;
    let mut failed = 0;
    let mut skipped = 0;
    for row in &result.rows {
        let chunk = ChunkArtifact {
            chunk_id: row[0].get_str().unwrap_or("").to_string(),
            document_id: row[1].get_str().unwrap_or("").to_string(),
            artifact_id: row[2].get_str().unwrap_or("").to_string(),
            chunk_index: row[3].get_int().unwrap_or(0) as u32,
            page_start: row[4].get_int().unwrap_or(0) as u32,
            page_end: row[5].get_int().unwrap_or(0) as u32,
            content: row[6].get_str().unwrap_or("").to_string(),
            content_hash: row[7].get_str().unwrap_or("").to_string(),
            embedding_status: row[8].get_str().unwrap_or("pending").to_string(),
        };

        // Skip already-indexed chunks when reindexing all
        if matches!(filter, ChunkFilter::All) && chunk.embedding_status == "indexed" {
            skipped += 1;
            continue;
        }

        match provider.embed_chunks(&[chunk.content.clone()]) {
            Ok(vectors) => {
                if let Some(embedding) = vectors.into_iter().next() {
                    if store::insert_chunk_embedding(
                        db,
                        &chunk.chunk_id,
                        &embedding,
                        provider.backend_name(),
                    )
                    .is_ok()
                    {
                        let _ =
                            store::update_chunk_embedding_status(db, &chunk.chunk_id, "indexed");
                        indexed += 1;
                    } else {
                        let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
                        failed += 1;
                    }
                } else {
                    let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
                    failed += 1;
                }
            }
            Err(_) => {
                let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
                failed += 1;
            }
        }
    }

    Ok(IndexResult {
        indexed,
        failed,
        skipped,
    })
}

/// Index all chunks with embedding_status = "pending".
pub fn index_pending_chunks(db: &DbInstance) -> Result<IndexResult, DocsError> {
    index_chunks_matching(db, ChunkFilter::OnlyPending)
}

/// Re-index all chunks regardless of status (useful after model swap).
pub fn reindex_all(db: &DbInstance) -> Result<IndexResult, DocsError> {
    index_chunks_matching(db, ChunkFilter::All)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::LocalEmbeddingProvider;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-retrieval-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

    /// Deterministic mock provider for tests (avoids loading fastembed).
    struct MockProvider {
        dim: usize,
    }

    impl LocalEmbeddingProvider for MockProvider {
        fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
            Ok(chunks
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    // Simple hash-based embedding: use byte values as seed
                    let mut v = vec![0.0_f32; self.dim];
                    for (j, b) in c.bytes().enumerate() {
                        v[j % self.dim] = (b as f32) / 255.0;
                    }
                    // Add a per-chunk offset so different chunks get different vectors
                    v[0] = (i as f32 + 1.0) * 0.5;
                    // L2-normalise
                    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
                    v.iter_mut().for_each(|x| *x /= norm);
                    v
                })
                .collect())
        }

        fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
            let chunks = vec![query.to_string()];
            let mut results = self.embed_chunks(&chunks)?;
            Ok(results.remove(0))
        }

        fn dimension(&self) -> usize {
            self.dim
        }

        fn backend_name(&self) -> &'static str {
            "mock"
        }
    }

    struct SynonymProvider;

    impl LocalEmbeddingProvider for SynonymProvider {
        fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
            Ok(chunks.iter().map(|text| fixture_vector(text)).collect())
        }

        fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
            Ok(fixture_vector(query))
        }

        fn dimension(&self) -> usize {
            4
        }

        fn backend_name(&self) -> &'static str {
            "mock-synonym"
        }
    }

    struct FailingQueryProvider;

    impl LocalEmbeddingProvider for FailingQueryProvider {
        fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
            Ok(chunks.iter().map(|text| fixture_vector(text)).collect())
        }

        fn embed_query(&self, _query: &str) -> Result<Vec<f32>, DocsError> {
            Err(DocsError::Embedding {
                message: "synthetic query embedding failure".into(),
            })
        }

        fn dimension(&self) -> usize {
            4
        }

        fn backend_name(&self) -> &'static str {
            "mock-query-failure"
        }
    }

    fn fixture_vector(text: &str) -> Vec<f32> {
        let lower = text.to_lowercase();
        let raw = if lower.contains("exact_only") {
            vec![-1.0, 0.0, 0.0, 0.0]
        } else if lower.contains("hybrid_target") {
            vec![0.8, 0.6, 0.0, 0.0]
        } else if lower.contains("car")
            || lower.contains("automobile")
            || lower.contains("vehicle")
            || lower.contains("market signal")
            || lower.contains("pure_semantic")
        {
            vec![1.0, 0.0, 0.0, 0.0]
        } else {
            vec![0.0, 1.0, 0.0, 0.0]
        };
        let norm = raw.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
        raw.into_iter().map(|x| x / norm).collect()
    }

    fn setup_with_provider(db: &DbInstance, dim: usize) {
        crate::schema::ensure_doc_schema(db).unwrap();
        crate::schema::ensure_vec_schema(db, dim).unwrap();
        crate::embed::set_provider(Box::new(MockProvider { dim }));
    }

    fn insert_test_chunk(db: &DbInstance, chunk_id: &str, content: &str) -> ChunkArtifact {
        let chunk = ChunkArtifact {
            chunk_id: chunk_id.into(),
            document_id: format!("doc-{chunk_id}"),
            artifact_id: format!("art-{chunk_id}"),
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
            content: content.into(),
            content_hash: format!("hash-{chunk_id}"),
            embedding_status: "pending".into(),
        };
        store::insert_chunk(db, &chunk).unwrap();
        chunk
    }

    #[test]
    fn test_search_empty_corpus() {
        let db = test_db();
        setup_with_provider(&db, 4);

        let results = search(&db, "test query", 5).unwrap();
        assert!(results.results.is_empty());
        assert_eq!(results.total_indexed_chunks, 0);
    }

    #[test]
    fn test_exact_search_finds_quoted_string() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        insert_test_chunk(
            &db,
            "chunk-quoted-hit",
            "The audit contains the exact phrase blue falcon protocol.",
        );
        insert_test_chunk(
            &db,
            "chunk-quoted-miss",
            "The audit contains unrelated policy text.",
        );

        let results = search_with_mode(
            &db,
            "\"blue falcon protocol\"",
            5,
            SearchMode::Exact,
            RetrievalWeights::default(),
        )
        .unwrap();

        assert_eq!(results.mode, SearchMode::Exact);
        assert!(!results.results.is_empty());
        assert_eq!(results.results[0].chunk_id, "chunk-quoted-hit");
        assert!(results.results[0].exact_score > 0.0);
        assert_eq!(results.results[0].semantic_score, 0.0);
    }

    #[test]
    fn test_exact_no_hit_reports_persisted_chunk_count() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        insert_test_chunk(
            &db,
            "chunk-existing",
            "Stored evidence about unrelated incentives.",
        );

        let results = search_with_mode(
            &db,
            "\"missing phrase\"",
            5,
            SearchMode::Exact,
            RetrievalWeights::default(),
        )
        .unwrap();

        assert!(results.results.is_empty());
        assert_eq!(results.total_chunks, 1);
        assert_eq!(results.total_indexed_chunks, 0);
    }

    #[test]
    fn test_semantic_search_finds_synonym() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        crate::schema::ensure_vec_schema(&db, 4).unwrap();
        crate::embed::set_provider(Box::new(SynonymProvider));
        let car = insert_test_chunk(
            &db,
            "chunk-car",
            "A car accelerates quickly through the junction.",
        );
        let fruit = insert_test_chunk(&db, "chunk-fruit", "Bananas ripen in warm storage rooms.");
        index_chunk(&db, &car).unwrap();
        index_chunk(&db, &fruit).unwrap();

        let results = search_with_mode(
            &db,
            "automobile",
            2,
            SearchMode::Semantic,
            RetrievalWeights::default(),
        )
        .unwrap();

        assert_eq!(results.mode, SearchMode::Semantic);
        assert_eq!(results.results[0].chunk_id, "chunk-car");
        assert!(results.results[0].semantic_score > 0.9);
        assert!(results.query_embedding_norm.unwrap_or(0.0) > 0.0);
    }

    #[test]
    fn test_hybrid_outperforms_either_alone_on_fixture() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        crate::schema::ensure_vec_schema(&db, 4).unwrap();
        crate::embed::set_provider(Box::new(SynonymProvider));
        let exact_only = insert_test_chunk(
            &db,
            "chunk-exact",
            "EXACT_ONLY market signal market signal unrelated ledger note.",
        );
        let semantic_only = insert_test_chunk(
            &db,
            "chunk-semantic",
            "PURE_SEMANTIC vehicle coalition dynamics with no query terms.",
        );
        let hybrid_target = insert_test_chunk(
            &db,
            "chunk-hybrid",
            "HYBRID_TARGET market cooperative transport alignment.",
        );
        index_chunk(&db, &exact_only).unwrap();
        index_chunk(&db, &semantic_only).unwrap();
        index_chunk(&db, &hybrid_target).unwrap();

        let weights = RetrievalWeights {
            exact: 0.5,
            semantic: 0.5,
        };
        let exact = search_with_mode(&db, "market signal", 3, SearchMode::Exact, weights).unwrap();
        let semantic =
            search_with_mode(&db, "market signal", 3, SearchMode::Semantic, weights).unwrap();
        let hybrid =
            search_with_mode(&db, "market signal", 3, SearchMode::Hybrid, weights).unwrap();

        assert_eq!(exact.results[0].chunk_id, "chunk-exact");
        assert_eq!(semantic.results[0].chunk_id, "chunk-semantic");
        assert_eq!(hybrid.results[0].chunk_id, "chunk-hybrid");
        assert!(hybrid.query_embedding_norm.unwrap_or(0.0) > 0.0);
        assert!(hybrid.results[0].exact_score > 0.0);
        assert!(hybrid.results[0].semantic_score > 0.0);
        assert!(hybrid.results[0].score > hybrid.results[1].score);
    }

    #[test]
    fn test_hybrid_propagates_real_embedding_errors() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        crate::schema::ensure_vec_schema(&db, 4).unwrap();
        crate::embed::set_provider(Box::new(SynonymProvider));
        let chunk = insert_test_chunk(&db, "chunk-vectorized", "A car market fixture.");
        index_chunk(&db, &chunk).unwrap();
        assert_eq!(store::count_embeddings(&db).unwrap(), 1);

        crate::embed::set_provider(Box::new(FailingQueryProvider));
        let err = search_with_mode(
            &db,
            "automobile",
            5,
            SearchMode::Hybrid,
            RetrievalWeights::default(),
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("synthetic query embedding failure")
        );
    }

    #[test]
    fn test_search_surfaces_failed_indexing() {
        let db = test_db();
        setup_with_provider(&db, 4);

        // Insert chunks with embedding_status = "failed" — no embeddings indexed
        let chunk1 = ChunkArtifact {
            chunk_id: "failed-chunk-a".into(),
            document_id: "doc-failed".into(),
            artifact_id: "art-1".into(),
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
            content: "content a".into(),
            content_hash: "hash-a".into(),
            embedding_status: "failed".into(),
        };
        let chunk2 = ChunkArtifact {
            chunk_id: "failed-chunk-b".into(),
            document_id: "doc-failed".into(),
            artifact_id: "art-1".into(),
            chunk_index: 1,
            page_start: 1,
            page_end: 1,
            content: "content b".into(),
            content_hash: "hash-b".into(),
            embedding_status: "failed".into(),
        };
        store::insert_chunk(&db, &chunk1).unwrap();
        store::insert_chunk(&db, &chunk2).unwrap();

        // No embeddings exist -> total == 0, but failed_count == 2
        let err = search(&db, "test query", 5).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("2 chunks failed"),
            "expected '2 chunks failed', got: {msg}"
        );
        assert!(
            msg.contains("archon docs model-status"),
            "expected diagnostic hint, got: {msg}"
        );
    }

    #[test]
    fn test_index_and_search_known_chunk() {
        let db = test_db();
        setup_with_provider(&db, 4);

        // Insert a chunk into doc_chunks
        let chunk = ChunkArtifact {
            chunk_id: "chunk-search-1".into(),
            document_id: "doc-search".into(),
            artifact_id: "art-1".into(),
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
            content: "The quick brown fox jumps over the lazy dog".into(),
            content_hash: "abc".into(),
            embedding_status: "pending".into(),
        };
        store::insert_chunk(&db, &chunk).unwrap();
        index_chunk(&db, &chunk).unwrap();

        let count = store::count_embeddings(&db).unwrap();
        assert_eq!(count, 1);

        // Search with the same text should return the chunk
        let results = search(&db, "quick brown fox", 5).unwrap();
        assert!(!results.results.is_empty(), "search should return results");
        assert_eq!(results.total_indexed_chunks, 1);
    }

    // ── HIGH #3: Search does not mutate ──────────────────────────────

    #[test]
    fn test_search_does_not_mutate() {
        let db = test_db();
        setup_with_provider(&db, 4);

        let chunk = ChunkArtifact {
            chunk_id: "chunk-no-mutate".into(),
            document_id: "doc-no-mutate".into(),
            artifact_id: "art-1".into(),
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
            content: "content for non-mutating search test".into(),
            content_hash: "h1".into(),
            embedding_status: "pending".into(),
        };
        store::insert_chunk(&db, &chunk).unwrap();

        let count_before = store::count_embeddings(&db).unwrap();
        assert_eq!(count_before, 0);

        let results = search(&db, "test query", 5).unwrap();
        assert!(
            !results.results.is_empty(),
            "hybrid search can return exact matches without indexing"
        );
        assert_eq!(results.total_indexed_chunks, 0);
        assert_eq!(results.results[0].chunk_id, "chunk-no-mutate");

        // Verify no mutation occurred
        let count_after = store::count_embeddings(&db).unwrap();
        assert_eq!(count_after, 0, "search must not create embeddings");

        let chunk_after = store::get_chunk_by_id(&db, "chunk-no-mutate")
            .unwrap()
            .unwrap();
        assert_eq!(
            chunk_after.embedding_status, "pending",
            "search must not change embedding status"
        );
    }

    // ── HIGH #3: Index only pending ──────────────────────────────────

    #[test]
    fn test_index_pending_command_indexes_pending_only() {
        let db = test_db();
        setup_with_provider(&db, 4);

        // Insert a chunk that's already "indexed" (but without actual embedding)
        let chunk_done = ChunkArtifact {
            chunk_id: "chunk-done".into(),
            document_id: "doc-mixed".into(),
            artifact_id: "art-1".into(),
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
            content: "already done chunk".into(),
            content_hash: "hd".into(),
            embedding_status: "indexed".into(),
        };
        store::insert_chunk(&db, &chunk_done).unwrap();

        // Insert a pending chunk
        let chunk_pending = ChunkArtifact {
            chunk_id: "chunk-pending".into(),
            document_id: "doc-mixed".into(),
            artifact_id: "art-2".into(),
            chunk_index: 1,
            page_start: 1,
            page_end: 1,
            content: "pending chunk for indexing".into(),
            content_hash: "hp".into(),
            embedding_status: "pending".into(),
        };
        store::insert_chunk(&db, &chunk_pending).unwrap();

        let result = index_pending_chunks(&db).unwrap();
        assert_eq!(
            result.indexed, 1,
            "only the pending chunk should be indexed"
        );
        assert_eq!(result.failed, 0);
        assert_eq!(result.skipped, 0);

        // Verify pending → indexed
        let pending_after = store::get_chunk_by_id(&db, "chunk-pending")
            .unwrap()
            .unwrap();
        assert_eq!(pending_after.embedding_status, "indexed");

        // Verify done chunk unchanged
        let done_after = store::get_chunk_by_id(&db, "chunk-done").unwrap().unwrap();
        assert_eq!(done_after.embedding_status, "indexed");

        // Only one embedding stored
        assert_eq!(store::count_embeddings(&db).unwrap(), 1);
    }

    // ── MEDIUM #4: get_chunk_by_id roundtrip ─────────────────────────

    #[test]
    fn test_get_chunk_by_id_roundtrip() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();

        let chunk = ChunkArtifact {
            chunk_id: "chunk-roundtrip-1".into(),
            document_id: "doc-roundtrip".into(),
            artifact_id: "art-1".into(),
            chunk_index: 2,
            page_start: 5,
            page_end: 7,
            content: "roundtrip test content".into(),
            content_hash: "rthash".into(),
            embedding_status: "pending".into(),
        };
        store::insert_chunk(&db, &chunk).unwrap();

        let found = store::get_chunk_by_id(&db, "chunk-roundtrip-1")
            .unwrap()
            .unwrap();
        assert_eq!(found.chunk_id, chunk.chunk_id);
        assert_eq!(found.document_id, chunk.document_id);
        assert_eq!(found.content, chunk.content);
        assert_eq!(found.page_start, 5);
        assert_eq!(found.page_end, 7);
        assert_eq!(found.chunk_index, 2);
        assert_eq!(found.content_hash, "rthash");
        assert_eq!(found.embedding_status, "pending");

        let not_found = store::get_chunk_by_id(&db, "no-such-chunk").unwrap();
        assert!(not_found.is_none());
    }

    // ── #6: IndexResult breakdown ──────────────────────────────────────

    struct SelectiveMockProvider {
        dim: usize,
    }

    impl LocalEmbeddingProvider for SelectiveMockProvider {
        fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
            chunks
                .iter()
                .map(|c| {
                    if c.contains("FAIL") {
                        Err(DocsError::Embedding {
                            message: "simulated embed failure".into(),
                        })
                    } else {
                        let mut v = vec![0.0_f32; self.dim];
                        for (j, b) in c.bytes().enumerate() {
                            v[j % self.dim] = (b as f32) / 255.0;
                        }
                        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
                        v.iter_mut().for_each(|x| *x /= norm);
                        Ok(v)
                    }
                })
                .collect()
        }

        fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
            let chunks = vec![query.to_string()];
            let results = self.embed_chunks(&chunks)?;
            Ok(results.into_iter().next().unwrap_or(vec![0.0; self.dim]))
        }

        fn dimension(&self) -> usize {
            self.dim
        }

        fn backend_name(&self) -> &'static str {
            "selective-mock"
        }
    }

    #[test]
    fn test_index_result_counts_indexed_and_failed() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        crate::embed::clear_provider();
        crate::embed::set_provider(Box::new(SelectiveMockProvider { dim: 4 }));

        // Chunk that will succeed
        let chunk_ok = ChunkArtifact {
            chunk_id: "chunk-ok".into(),
            document_id: "doc-result".into(),
            artifact_id: "art-1".into(),
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
            content: "normal content".into(),
            content_hash: "h1".into(),
            embedding_status: "pending".into(),
        };
        store::insert_chunk(&db, &chunk_ok).unwrap();

        // Chunk that will fail embedding (content contains "FAIL")
        let chunk_fail = ChunkArtifact {
            chunk_id: "chunk-fail".into(),
            document_id: "doc-result".into(),
            artifact_id: "art-2".into(),
            chunk_index: 1,
            page_start: 2,
            page_end: 2,
            content: "content with FAIL trigger".into(),
            content_hash: "h2".into(),
            embedding_status: "pending".into(),
        };
        store::insert_chunk(&db, &chunk_fail).unwrap();

        let result = index_pending_chunks(&db).unwrap();
        assert_eq!(result.indexed, 1, "one chunk should succeed");
        assert_eq!(result.failed, 1, "one chunk should fail");
        assert_eq!(
            result.skipped, 0,
            "no chunks should be skipped in pending-only mode"
        );

        // Verify the failed chunk's status was updated
        let failed_after = store::get_chunk_by_id(&db, "chunk-fail").unwrap().unwrap();
        assert_eq!(failed_after.embedding_status, "failed");
    }

    #[test]
    fn test_reindex_all_counts_skipped() {
        let db = test_db();
        setup_with_provider(&db, 4);

        // Insert a chunk already marked "indexed"
        let chunk_indexed = ChunkArtifact {
            chunk_id: "chunk-indexed".into(),
            document_id: "doc-skip".into(),
            artifact_id: "art-1".into(),
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
            content: "already indexed".into(),
            content_hash: "hi".into(),
            embedding_status: "indexed".into(),
        };
        store::insert_chunk(&db, &chunk_indexed).unwrap();

        // Insert a pending chunk
        let chunk_pending = ChunkArtifact {
            chunk_id: "chunk-pending2".into(),
            document_id: "doc-skip".into(),
            artifact_id: "art-2".into(),
            chunk_index: 1,
            page_start: 2,
            page_end: 2,
            content: "pending work".into(),
            content_hash: "hp".into(),
            embedding_status: "pending".into(),
        };
        store::insert_chunk(&db, &chunk_pending).unwrap();

        let result = reindex_all(&db).unwrap();
        assert_eq!(result.indexed, 1, "pending chunk should be indexed");
        assert_eq!(result.failed, 0);
        assert_eq!(result.skipped, 1, "already-indexed chunk should be skipped");
    }
}
