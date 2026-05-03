//! CozoDB HNSW retrieval for document chunks.
//!
//! Provides the search pipeline: embed query → HNSW nearest-neighbour →
//! resolve chunks from doc_chunks → optional reranking → results with
//! provenance.

use std::collections::BTreeMap;

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
}

/// Batch of search results returned from a query.
#[derive(Clone, Debug)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
    pub query: String,
    pub total_indexed_chunks: usize,
}

/// Search for chunks similar to `query` using HNSW vector search.
///
/// If no embedding provider is configured, returns `DocsError::ModelNotConfigured`.
/// If no chunks are indexed, returns an empty result set.
pub fn search(
    db: &DbInstance,
    query: &str,
    top_k: usize,
) -> Result<SearchResults, DocsError> {
    let provider = get_provider().ok_or_else(|| DocsError::ModelNotConfigured {
        message: "no embedding provider configured. Run 'archon docs model-status' for details."
            .into(),
    })?;

    let total = store::count_embeddings(db).map_err(|e| DocsError::Retrieval {
        message: e.to_string(),
    })?;

    if total == 0 {
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
            total_indexed_chunks: 0,
        });
    }

    let query_vec = provider.embed_query(query)?;

    let results = hnsw_search(db, &query_vec, top_k)?;

    Ok(SearchResults {
        results,
        query: query.to_string(),
        total_indexed_chunks: total,
    })
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

        search_results.push(SearchResult {
            score: 1.0 - distance / 2.0,
            distance,
            chunk_id,
            document_id: chunk.document_id,
            content: chunk.content,
            page_start: chunk.page_start,
            page_end: chunk.page_end,
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
pub fn index_chunk(
    db: &DbInstance,
    chunk: &ChunkArtifact,
) -> Result<(), DocsError> {
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
                    ).is_ok() {
                        let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "indexed");
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

    Ok(IndexResult { indexed, failed, skipped })
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

    fn setup_with_provider(db: &DbInstance, dim: usize) {
        crate::schema::ensure_doc_schema(db).unwrap();
        crate::schema::ensure_vec_schema(db, dim).unwrap();
        crate::embed::set_provider(Box::new(MockProvider { dim }));
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
        assert!(msg.contains("2 chunks failed"), "expected '2 chunks failed', got: {msg}");
        assert!(msg.contains("archon docs model-status"), "expected diagnostic hint, got: {msg}");
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
        assert!(results.results.is_empty());
        assert_eq!(results.total_indexed_chunks, 0);

        // Verify no mutation occurred
        let count_after = store::count_embeddings(&db).unwrap();
        assert_eq!(count_after, 0, "search must not create embeddings");

        let chunk_after = store::get_chunk_by_id(&db,"chunk-no-mutate").unwrap().unwrap();
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
        assert_eq!(result.indexed, 1, "only the pending chunk should be indexed");
        assert_eq!(result.failed, 0);
        assert_eq!(result.skipped, 0);

        // Verify pending → indexed
        let pending_after = store::get_chunk_by_id(&db,"chunk-pending").unwrap().unwrap();
        assert_eq!(pending_after.embedding_status, "indexed");

        // Verify done chunk unchanged
        let done_after = store::get_chunk_by_id(&db,"chunk-done").unwrap().unwrap();
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

        let found = store::get_chunk_by_id(&db, "chunk-roundtrip-1").unwrap().unwrap();
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
        assert_eq!(result.skipped, 0, "no chunks should be skipped in pending-only mode");

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
