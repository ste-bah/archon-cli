use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::graph::{raw_to_memory, read_all_memories, row_values_to_memory};
use crate::types::{Memory, MemoryError, SearchFilter};

pub(crate) const FULL_SCAN_WARNING_THRESHOLD: usize = 10_000;
const INITIAL_FTS_LIMIT: i64 = 256;
const MEMORY_COLUMNS: &str = "id, content, title, memory_type, importance, tags, source_type, project_path, created_at, updated_at, access_count, last_accessed";

pub(crate) struct KeywordCandidates {
    pub(crate) memories: Vec<Memory>,
    #[cfg(test)]
    pub(crate) used_fts: bool,
}

pub(crate) fn keyword_candidates(
    db: &DbInstance,
    query: &str,
    limit: usize,
) -> Result<KeywordCandidates, MemoryError> {
    if query.split_whitespace().next().is_none() || limit == 0 {
        return Ok(KeywordCandidates {
            memories: Vec::new(),
            #[cfg(test)]
            used_fts: true,
        });
    }

    match fts_keyword_candidates(db, query) {
        Ok(memories) => Ok(KeywordCandidates {
            memories,
            #[cfg(test)]
            used_fts: true,
        }),
        Err(MemoryError::Database(message)) if fts_index_unavailable(&message) => {
            let all_rows = read_all_memories(db)?;
            warn_full_scan("memory.keyword.fallback", all_rows.len(), Some(limit));
            Ok(KeywordCandidates {
                memories: all_rows
                    .into_iter()
                    .filter_map(|row| raw_to_memory(row).ok())
                    .collect(),
                #[cfg(test)]
                used_fts: false,
            })
        }
        Err(error) => Err(error),
    }
}

fn fts_keyword_candidates(db: &DbInstance, query: &str) -> Result<Vec<Memory>, MemoryError> {
    let mut memories = Vec::new();
    let mut seen = BTreeSet::new();

    for index in ["content_fts", "title_fts", "tags_fts"] {
        let mut limit = INITIAL_FTS_LIMIT;
        loop {
            let result = fts_candidate_query(db, query, index, limit)?;
            let exhausted = result.rows.len() < limit as usize;
            for row in result.rows {
                if let Some(id) = row.get(1).and_then(DataValue::get_str)
                    && seen.insert(id.to_string())
                {
                    memories.push(row_values_to_memory(&row[1..])?);
                }
            }
            if exhausted {
                break;
            }
            limit = limit.checked_mul(2).ok_or_else(|| {
                MemoryError::Database("keyword candidate limit exceeds i64".into())
            })?;
        }
    }
    Ok(memories)
}

fn fts_candidate_query(
    db: &DbInstance,
    query: &str,
    index: &str,
    limit: i64,
) -> Result<cozo::NamedRows, MemoryError> {
    let mut params = BTreeMap::new();
    params.insert("query".into(), DataValue::from(fts_query(query)));
    params.insert("limit".into(), DataValue::from(limit));
    let script = format!(
        "?[score, {MEMORY_COLUMNS}] := ~memories:{index} {{{MEMORY_COLUMNS} | query: $query, k: $limit, score_kind: 'tf_idf', bind_score: score }} :order -score"
    );
    db.run_script(&script, params, ScriptMutability::Immutable)
        .map_err(|error| MemoryError::Database(error.to_string()))
}

fn fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|term| format!("{:?}", term))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn fts_index_unavailable(message: &str) -> bool {
    message.contains("Index ") && message.contains(" not found on relation ")
}

pub(crate) fn full_scan_contract(
    surface: &str,
    row_count: usize,
    limit: Option<usize>,
) -> Option<String> {
    if row_count < FULL_SCAN_WARNING_THRESHOLD {
        return None;
    }
    Some(match limit {
        Some(limit) => format!(
            "{surface} is using a keyword/full-scan memory path over {row_count} rows before returning at most {limit}; attach an embedding provider for indexed vector narrowing or expect latency to scale with memory count"
        ),
        None => format!(
            "{surface} is using a keyword/full-scan memory path over {row_count} rows; attach an embedding provider or expect latency to scale with memory count"
        ),
    })
}

pub(crate) fn warn_full_scan(surface: &str, row_count: usize, limit: Option<usize>) {
    if let Some(message) = full_scan_contract(surface, row_count, limit) {
        tracing::warn!(
            surface,
            row_count,
            limit = limit.unwrap_or_default(),
            "{message}"
        );
    }
}

/// Keyword-based recall with relevance ranking.
///
/// Scoring formula per memory:
///   score = keyword_hits * 10
///         + tag_hits * 5
///         + recency_boost   (newer = higher, max 3.0)
///         + access_boost    (log2(access_count + 1), max 3.0)
pub(crate) fn recall(
    db: &DbInstance,
    query: &str,
    limit: usize,
) -> Result<Vec<Memory>, MemoryError> {
    let keywords: Vec<&str> = query.split_whitespace().collect();
    if keywords.is_empty() {
        return Ok(Vec::new());
    }

    let candidates = keyword_candidates(db, query, limit)?;
    let now = Utc::now();

    let mut scored: Vec<(f64, Memory)> = Vec::new();

    for mem in candidates.memories {
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

        if !matched {
            continue;
        }

        // Recency boost: days old, capped contribution.
        let age_days = (now - mem.created_at).num_days().max(0) as f64;
        let recency = 3.0 / (1.0 + age_days * 0.1);

        let access_boost = (mem.access_count as f64 + 1.0).log2().min(3.0);

        let score = keyword_hits * 10.0 + tag_hits * 5.0 + recency + access_boost;

        scored.push((score, mem));
    }

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);

    Ok(scored.into_iter().map(|(_, m)| m).collect())
}

fn structured_search_candidates(
    db: &DbInstance,
    filter: &SearchFilter,
) -> Result<KeywordCandidates, MemoryError> {
    if let Some(text) = filter.text.as_deref() {
        return keyword_candidates(db, text, usize::MAX);
    }
    if !filter.tags.is_empty() {
        return keyword_candidates(db, &filter.tags.join(" "), usize::MAX);
    }
    let all_rows = read_all_memories(db)?;
    warn_full_scan("memory.search.filter", all_rows.len(), None);
    Ok(KeywordCandidates {
        memories: all_rows
            .into_iter()
            .filter_map(|row| raw_to_memory(row).ok())
            .collect(),
        #[cfg(test)]
        used_fts: false,
    })
}

/// Structured search with filters (type, tags, text, date range).
pub(crate) fn search(db: &DbInstance, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
    let has_any_filter = filter.memory_type.is_some()
        || filter.text.is_some()
        || !filter.tags.is_empty()
        || filter.date_from.is_some()
        || filter.date_to.is_some();

    if !has_any_filter {
        return Ok(Vec::new());
    }

    let candidates = structured_search_candidates(db, filter)?;
    let mut results: Vec<Memory> = Vec::new();

    for mem in candidates.memories {
        // Filter by memory_type
        if let Some(mt) = filter.memory_type
            && mem.memory_type != mt
        {
            continue;
        }

        // Filter by text (case-insensitive substring in content or title)
        if let Some(ref text) = filter.text {
            let text_lower = text.to_lowercase();
            let content_lower = mem.content.to_lowercase();
            let title_lower = mem.title.to_lowercase();
            if !content_lower.contains(&text_lower) && !title_lower.contains(&text_lower) {
                continue;
            }
        }

        // Filter by date_from
        if let Some(ref from) = filter.date_from
            && mem.created_at < *from
        {
            continue;
        }

        // Filter by date_to
        if let Some(ref to) = filter.date_to
            && mem.created_at > *to
        {
            continue;
        }

        // Filter by tags
        if !filter.tags.is_empty() {
            if filter.require_all_tags {
                if !filter.tags.iter().all(|ft| mem.tags.contains(ft)) {
                    continue;
                }
            } else if !filter.tags.iter().any(|ft| mem.tags.contains(ft)) {
                continue;
            }
        }

        results.push(mem);
    }

    // Sort by created_at descending (newest first).
    results.sort_by_key(|b| std::cmp::Reverse(b.created_at));

    Ok(results)
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
