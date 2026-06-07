use std::collections::{BTreeMap, HashMap};

use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::errors::DocsError;
use crate::models::ChunkArtifact;
use crate::retrieval::SearchResult;
use crate::retrieval_query::{normalized_exact_text, safe_fts_query};

const EXACT_CANDIDATE_MULTIPLIER: usize = 3;
const EXACT_SCAN_FALLBACK_MAX_CHUNKS: usize = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExactFallback {
    Strict,
    BestEffort,
}

pub(crate) fn exact_search(
    db: &DbInstance,
    query: &str,
    top_k: usize,
    total_chunks: usize,
    fallback: ExactFallback,
) -> Result<Vec<SearchResult>, DocsError> {
    let needle = normalize_exact_query(query);
    if needle.is_empty() || top_k == 0 {
        return Ok(Vec::new());
    }

    let candidate_limit = expanded_top_k(top_k);
    let fts_query = safe_fts_query(&needle);
    if fts_query.is_empty() {
        return Ok(Vec::new());
    }
    match fts_search(db, &fts_query, candidate_limit) {
        Ok(results) if !results.is_empty() => return rank_exact_results(results, top_k),
        Ok(_) => {}
        Err(e) => {
            if total_chunks > EXACT_SCAN_FALLBACK_MAX_CHUNKS && fallback == ExactFallback::Strict {
                return Err(DocsError::Retrieval {
                    message: format!(
                        "FTS search failed and exact scan fallback is capped at {EXACT_SCAN_FALLBACK_MAX_CHUNKS} chunks: {e}"
                    ),
                });
            }
            tracing::warn!(error = %e, "Cozo FTS search failed; using bounded exact fallback");
        }
    }

    if total_chunks > EXACT_SCAN_FALLBACK_MAX_CHUNKS {
        tracing::warn!(
            total_chunks,
            max_fallback_chunks = EXACT_SCAN_FALLBACK_MAX_CHUNKS,
            "skipping unbounded exact scan fallback"
        );
        return Ok(Vec::new());
    }

    let scan_results = scan_exact_matches(db, &needle, candidate_limit, total_chunks)?;
    rank_exact_results(scan_results, top_k)
}

pub(crate) fn expanded_top_k(top_k: usize) -> usize {
    top_k.saturating_mul(EXACT_CANDIDATE_MULTIPLIER).max(top_k)
}

pub(crate) fn score_exact_for_candidates(
    query: &str,
    candidates: &mut HashMap<String, SearchResult>,
) {
    let query_lower = normalize_exact_query(query).to_lowercase();
    let query_terms = exact_query_terms(&query_lower);
    for result in candidates.values_mut() {
        if let Some(score) = exact_content_score(&query_lower, &query_terms, &result.content)
            && score > result.exact_score
        {
            result.exact_score = score;
        }
    }
}

pub(crate) fn relaxed_exact_search(
    db: &DbInstance,
    query: &str,
    top_k: usize,
) -> Result<Vec<SearchResult>, DocsError> {
    if top_k == 0 {
        return Ok(Vec::new());
    }
    for term in relaxed_terms(query) {
        let results = fts_search(db, &term, expanded_top_k(top_k))?;
        if !results.is_empty() {
            return rank_exact_results(results, top_k);
        }
    }
    Ok(Vec::new())
}

pub(crate) fn list_chunks_for_exact_fallback(
    db: &DbInstance,
    max_chunks: usize,
) -> Result<Vec<ChunkArtifact>, DocsError> {
    if max_chunks == 0 {
        return Ok(Vec::new());
    }
    let mut params = BTreeMap::new();
    params.insert("limit".to_string(), DataValue::from(max_chunks as i64));
    let result = db
        .run_script(
            "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] \
             := *doc_chunks{chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status} \
             :limit $limit",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| DocsError::Retrieval {
            message: format!("list chunks for bounded exact fallback failed: {e}"),
        })?;
    Ok(result.rows.iter().map(|row| chunk_from_row(row)).collect())
}

fn relaxed_terms(query: &str) -> Vec<String> {
    safe_fts_query(query)
        .split_whitespace()
        .filter(|term| term.len() >= 5)
        .take(4)
        .map(ToString::to_string)
        .collect()
}

fn rank_exact_results(
    results: Vec<SearchResult>,
    top_k: usize,
) -> Result<Vec<SearchResult>, DocsError> {
    let mut merged: HashMap<String, SearchResult> = HashMap::new();
    for result in results {
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
    max_chunks: usize,
) -> Result<Vec<SearchResult>, DocsError> {
    let query_lower = query.to_lowercase();
    let query_terms = exact_query_terms(&query_lower);
    let mut scored: Vec<(ChunkArtifact, f64)> = list_chunks_for_exact_fallback(db, max_chunks)?
        .into_iter()
        .filter_map(|chunk| {
            exact_content_score(&query_lower, &query_terms, &chunk.content)
                .map(|score| (chunk, score))
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k);
    Ok(scored
        .into_iter()
        .map(|(chunk, score)| exact_result_from_chunk(chunk, score))
        .collect())
}

fn exact_query_terms(query_lower: &str) -> Vec<&str> {
    query_lower.split_whitespace().collect()
}

fn exact_content_score(query_lower: &str, query_terms: &[&str], content: &str) -> Option<f64> {
    let content_lower = content.to_lowercase();
    let count = content_lower.matches(query_lower).count();
    let matched_terms = query_terms
        .iter()
        .filter(|term| content_lower.contains(**term))
        .count();
    if count == 0 && matched_terms == 0 {
        return None;
    }
    let term_score = if query_terms.is_empty() {
        0.0
    } else {
        matched_terms as f64 / query_terms.len() as f64
    };
    let occurrence_score = (count as f64 / 3.0).min(1.0);
    let score = if count == 0 {
        term_score * 0.7
    } else {
        (term_score + occurrence_score) / 2.0
    };
    Some(score.clamp(0.0, 1.0))
}

fn normalize_exact_query(query: &str) -> String {
    normalized_exact_text(query)
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
