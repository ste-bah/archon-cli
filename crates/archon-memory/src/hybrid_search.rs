//! Hybrid search: merge vector similarity and keyword relevance scores.
//!
//! The `alpha` parameter controls the blend:
//! - `alpha = 0.0` → pure vector search
//! - `alpha = 1.0` → pure keyword search
//! - `alpha = 0.3` (default) → 30% keyword + 70% vector

use std::collections::HashMap;

use cozo::DbInstance;

use crate::embedding::EmbeddingProvider;
use crate::graph::{raw_to_memory, read_all_memories};
use crate::types::{Memory, MemoryError};
use crate::vector_search;

/// Perform a hybrid keyword + vector search and return ranked memories.
///
/// When `alpha` is 1.0, only keyword search runs (no vector ops needed).
/// When `alpha` is 0.0, only vector search runs.
pub fn hybrid_search(
    db: &DbInstance,
    query: &str,
    provider: &dyn EmbeddingProvider,
    alpha: f32,
    top_k: usize,
) -> Result<Vec<Memory>, MemoryError> {
    let alpha = alpha.clamp(0.0, 1.0);

    // Phase 1: keyword scores (always compute for the merge, unless alpha=0)
    let keyword_scores: HashMap<String, f64> = if alpha > 0.0 {
        compute_keyword_scores(db, query)?
    } else {
        HashMap::new()
    };

    // Phase 2: vector scores (unless alpha=1)
    let vector_scores: HashMap<String, f64> = if alpha < 1.0 {
        compute_vector_scores(db, query, provider, top_k)?
    } else {
        HashMap::new()
    };

    // Merge: collect all candidate IDs
    let mut all_ids: HashMap<String, f64> = HashMap::new();

    for (id, kw_score) in &keyword_scores {
        let vs = vector_scores.get(id).copied().unwrap_or(0.0);
        let final_score = (alpha as f64) * kw_score + (1.0 - alpha as f64) * vs;
        all_ids.insert(id.clone(), final_score);
    }

    for (id, vs) in &vector_scores {
        if !all_ids.contains_key(id) {
            let kw = keyword_scores.get(id).copied().unwrap_or(0.0);
            let final_score = (alpha as f64) * kw + (1.0 - alpha as f64) * vs;
            all_ids.insert(id.clone(), final_score);
        }
    }

    // Sort by score descending
    let mut ranked: Vec<(String, f64)> = all_ids.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(top_k);

    // Fetch full Memory objects
    let all_rows = read_all_memories(db)?;
    let mut memory_map: HashMap<String, Memory> = HashMap::new();
    for raw in all_rows {
        if let Ok(mem) = raw_to_memory(raw) {
            memory_map.insert(mem.id.clone(), mem);
        }
    }

    let mut results = Vec::with_capacity(ranked.len());
    for (id, _score) in ranked {
        if let Some(mem) = memory_map.remove(&id) {
            results.push(mem);
        }
    }

    Ok(results)
}

/// Compute normalised keyword relevance scores for all memories.
///
/// Re-uses the same scoring logic as `search::recall` but returns a
/// normalised `[0, 1]` score map.
fn compute_keyword_scores(
    db: &DbInstance,
    query: &str,
) -> Result<HashMap<String, f64>, MemoryError> {
    let keywords: Vec<&str> = query.split_whitespace().collect();
    if keywords.is_empty() {
        return Ok(HashMap::new());
    }

    let all_rows = read_all_memories(db)?;
    let mut scores: Vec<(String, f64)> = Vec::new();

    for raw in all_rows {
        let mem = match raw_to_memory(raw) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let content_lower = mem.content.to_lowercase();
        let tags_lower = mem.tags.join(",").to_lowercase();
        let title_lower = mem.title.to_lowercase();

        let mut keyword_hits: f64 = 0.0;
        let mut tag_hits: f64 = 0.0;
        let mut matched = false;

        for kw in &keywords {
            let kw_lower = kw.to_lowercase();
            if content_lower.contains(&kw_lower) {
                keyword_hits += 1.0;
                matched = true;
            }
            if title_lower.contains(&kw_lower) {
                keyword_hits += 1.0;
                matched = true;
            }
            if tags_lower.contains(&kw_lower) {
                tag_hits += 1.0;
                matched = true;
            }
        }

        if matched {
            let score = keyword_hits * 10.0 + tag_hits * 5.0;
            scores.push((mem.id, score));
        }
    }

    // Normalise to [0, 1]
    let max_score = scores.iter().map(|(_, s)| *s).fold(0.0f64, f64::max);
    let mut map = HashMap::with_capacity(scores.len());
    for (id, score) in scores {
        let normalised = if max_score > 0.0 {
            score / max_score
        } else {
            0.0
        };
        map.insert(id, normalised);
    }

    Ok(map)
}

/// Compute vector similarity scores for the top candidates.
///
/// Returns `(memory_id, similarity)` where similarity = `1.0 - cosine_distance`.
fn compute_vector_scores(
    db: &DbInstance,
    query: &str,
    provider: &dyn EmbeddingProvider,
    top_k: usize,
) -> Result<HashMap<String, f64>, MemoryError> {
    let query_embedding = provider.embed(&[query.to_string()])?;
    if query_embedding.is_empty() {
        return Ok(HashMap::new());
    }

    // Search may fail if the HNSW index doesn't exist yet (no embeddings stored)
    let results = match vector_search::search_similar(db, &query_embedding[0], top_k) {
        Ok(r) => r,
        Err(_) => return Ok(HashMap::new()),
    };

    let mut map = HashMap::with_capacity(results.len());
    for (id, distance) in results {
        // Convert cosine distance to similarity: sim = 1.0 - distance
        let similarity = (1.0 - distance).max(0.0);
        map.insert(id, similarity);
    }

    Ok(map)
}
