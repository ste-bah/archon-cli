use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability, Vector};

use crate::embed::{get_provider, init_default_provider};
use crate::errors::DocsError;
use crate::retrieval::SearchResult;
use crate::store;
use crate::vector_store::DocVectorStore;

#[derive(Debug)]
pub(crate) struct SemanticSearch {
    pub(crate) results: Vec<SearchResult>,
    pub(crate) query_embedding_norm: f64,
}

pub(crate) fn semantic_search_with_norm(
    db: &DbInstance,
    query: &str,
    top_k: usize,
) -> Result<SemanticSearch, DocsError> {
    let provider = get_or_init_provider().ok_or_else(|| DocsError::ModelNotConfigured {
        message: "no embedding provider configured. Run 'archon docs model-status' for details."
            .into(),
    })?;
    let provider_name = provider.backend_name();
    let query_vec = provider.embed_query(query)?;
    let query_embedding_norm = l2_norm(&query_vec);
    let results = hnsw_search(db, provider_name, &query_vec, top_k)?;
    Ok(SemanticSearch {
        results,
        query_embedding_norm,
    })
}

fn get_or_init_provider() -> Option<std::sync::Arc<dyn crate::embed::LocalEmbeddingProvider>> {
    if get_provider().is_none()
        && let Err(error) = init_default_provider()
    {
        tracing::warn!(%error, "docs embedding provider not available");
    }
    get_provider()
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

fn hnsw_search(
    db: &DbInstance,
    provider_name: &str,
    query_vec: &[f32],
    top_k: usize,
) -> Result<Vec<SearchResult>, DocsError> {
    if let Some(results) = rocksdb_hnsw_search(db, provider_name, query_vec, top_k)? {
        return Ok(results);
    }
    legacy_cozo_hnsw_search(db, query_vec, top_k)
}

fn rocksdb_hnsw_search(
    db: &DbInstance,
    provider_name: &str,
    query_vec: &[f32],
    top_k: usize,
) -> Result<Option<Vec<SearchResult>>, DocsError> {
    let vector_store = match DocVectorStore::open_default() {
        Ok(store) => store,
        Err(error) => {
            tracing::warn!(%error, "RocksDB vector store unavailable; trying legacy Cozo HNSW");
            return Ok(None);
        }
    };
    let raw_count = vector_store
        .count_vectors(Some(provider_name))
        .map_err(|e| DocsError::Retrieval {
            message: format!("RocksDB vector count failed: {e}"),
        })?;
    if raw_count == 0 {
        return Ok(None);
    }
    let hits = vector_store
        .search_in_memory(provider_name, query_vec, top_k, 50, None)
        .map_err(|e| DocsError::Retrieval {
            message: format!("Rust-HNSW search failed: {e}"),
        })?;
    let mut search_results = resolve_vector_hits(
        db,
        hits.into_iter()
            .map(|hit| (hit.chunk_id, f64::from(hit.distance))),
    )?;
    sort_by_distance(&mut search_results);
    Ok(Some(search_results))
}

fn legacy_cozo_hnsw_search(
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

    let mut search_results = resolve_vector_hits(
        db,
        result.rows.iter().map(|row| {
            (
                row[0].get_str().unwrap_or("").to_string(),
                row[1].get_float().unwrap_or(1.0),
            )
        }),
    )?;
    sort_by_distance(&mut search_results);
    Ok(search_results)
}

fn resolve_vector_hits(
    db: &DbInstance,
    hits: impl IntoIterator<Item = (String, f64)>,
) -> Result<Vec<SearchResult>, DocsError> {
    let mut search_results = Vec::new();
    for (chunk_id, distance) in hits {
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
    Ok(search_results)
}

fn sort_by_distance(search_results: &mut [SearchResult]) {
    search_results.sort_by(|a, b| {
        a.distance
            .partial_cmp(&b.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}
