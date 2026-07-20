use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::graph::{raw_to_memory, read_all_memories, row_values_to_memory};
use crate::types::{Memory, MemoryError, SearchFilter};

pub(crate) const FULL_SCAN_WARNING_THRESHOLD: usize = 10_000;
const KEYWORD_CANDIDATE_MULTIPLIER: usize = 4;
const MIN_KEYWORD_CANDIDATES: usize = 256;
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

    let candidate_limit = limit
        .saturating_mul(KEYWORD_CANDIDATE_MULTIPLIER)
        .max(MIN_KEYWORD_CANDIDATES);
    match fts_keyword_candidates(db, query, candidate_limit) {
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

fn fts_keyword_candidates(
    db: &DbInstance,
    query: &str,
    limit: usize,
) -> Result<Vec<Memory>, MemoryError> {
    let limit = i64::try_from(limit)
        .map_err(|_| MemoryError::Database("keyword candidate limit exceeds i64".into()))?;
    let mut params = BTreeMap::new();
    params.insert("query".into(), DataValue::from(fts_query(query)));
    params.insert("limit".into(), DataValue::from(limit));

    let mut memories = Vec::new();
    let mut seen = BTreeSet::new();
    for index in ["content_fts", "title_fts", "tags_fts"] {
        let script = format!(
            "?[score, {MEMORY_COLUMNS}] := ~memories:{index} {{{MEMORY_COLUMNS} | query: $query, k: $limit, score_kind: 'tf_idf', bind_score: score }} :order -score"
        );
        let result = db
            .run_script(&script, params.clone(), ScriptMutability::Immutable)
            .map_err(|error| MemoryError::Database(error.to_string()))?;
        for row in result.rows {
            if let Some(id) = row.get(1).and_then(DataValue::get_str)
                && seen.insert(id.to_string())
            {
                memories.push(row_values_to_memory(&row[1..])?);
            }
        }
    }
    Ok(memories)
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

/// Structured search with filters (type, tags, text, date range).
pub(crate) fn search(db: &DbInstance, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
    // If no filters are set, return empty (same behavior as original).
    let has_any_filter = filter.memory_type.is_some()
        || filter.text.is_some()
        || !filter.tags.is_empty()
        || filter.date_from.is_some()
        || filter.date_to.is_some();

    if !has_any_filter {
        return Ok(Vec::new());
    }

    // Fetch all memories and filter in Rust.
    let all_rows = read_all_memories(db)?;
    warn_full_scan("memory.search.filter", all_rows.len(), None);

    let mut results: Vec<Memory> = Vec::new();

    for raw in all_rows {
        let mem = match raw_to_memory(raw) {
            Ok(m) => m,
            Err(_) => continue,
        };

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
mod tests {
    use super::*;
    use crate::graph::MemoryGraph;
    use crate::types::MemoryType;

    #[test]
    fn keyword_candidates_use_fts_for_content_title_and_tags() {
        let g = MemoryGraph::in_memory().expect("graph creation failed");
        let content_id = g
            .store_memory("indexed-content-marker", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");
        let title_id = g
            .store_memory("body", "indexed-title-marker", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");
        let tag_id = g
            .store_memory(
                "body",
                "",
                MemoryType::Fact,
                0.5,
                &["indexed-tag-marker".to_string()],
                "m",
                "",
            )
            .expect("store failed");

        for (query, expected_id) in [
            ("indexed-content-marker", content_id),
            ("indexed-title-marker", title_id),
            ("indexed-tag-marker", tag_id),
        ] {
            let candidates = keyword_candidates(g.db(), query, 16).expect("FTS query failed");
            assert!(candidates.used_fts);
            assert!(candidates.memories.iter().any(|memory| memory.id == expected_id));
        }
    }

    #[test]
    fn keyword_fts_tracks_updates_and_deletes() {
        let g = MemoryGraph::in_memory().expect("graph creation failed");
        let id = g
            .store_memory("before-update-marker", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");

        g.update_memory(&id, Some("after-update-marker"), None)
            .expect("update failed");
        let updated = keyword_candidates(g.db(), "after-update-marker", 16)
            .expect("updated FTS query failed");
        assert!(updated.used_fts);
        assert!(updated.memories.iter().any(|memory| memory.id == id));

        g.delete_memory(&id).expect("delete failed");
        let deleted = keyword_candidates(g.db(), "after-update-marker", 16)
            .expect("deleted FTS query failed");
        assert!(deleted.used_fts);
        assert!(deleted.memories.iter().all(|memory| memory.id != id));
    }

    #[test]
    fn keyword_candidates_fall_back_when_fts_is_unavailable() {
        let db = DbInstance::new("mem", "", "").expect("db creation failed");
        db.run_script(
            ":create memories {
                id: String => content: String, title: String, memory_type: String,
                importance: Float, tags: String, source_type: String, project_path: String,
                created_at: String, updated_at: String, access_count: Int, last_accessed: String
            }",
            Default::default(),
            cozo::ScriptMutability::Mutable,
        )
        .expect("relation creation failed");
        let now = Utc::now().to_rfc3339();
        let params = std::collections::BTreeMap::from([
            ("id".to_string(), cozo::DataValue::from("fallback-id")),
            ("now".to_string(), cozo::DataValue::from(now.as_str())),
        ]);
        db.run_script(
            "?[id, content, title, memory_type, importance, tags, source_type, project_path,
                created_at, updated_at, access_count, last_accessed] <- [[
                $id, 'fallback-marker', '', 'fact', 0.5, '[]', 'test', '', $now, '', 0, ''
            ]] :put memories {id => content, title, memory_type, importance, tags, source_type,
                project_path, created_at, updated_at, access_count, last_accessed}",
            params,
            cozo::ScriptMutability::Mutable,
        )
        .expect("memory insert failed");

        let candidates = keyword_candidates(&db, "fallback-marker", 16)
            .expect("fallback query failed");
        assert!(!candidates.used_fts);
        assert_eq!(candidates.memories.len(), 1);
        assert_eq!(candidates.memories[0].id, "fallback-id");
    }

    #[test]
    fn keyword_candidates_match_any_query_term() {
        let g = MemoryGraph::in_memory().expect("graph creation failed");
        let rust_id = g
            .store_memory("rust", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");
        let python_id = g
            .store_memory("python", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");

        let candidates = keyword_candidates(g.db(), "rust python", 16).expect("FTS query failed");
        assert!(candidates.memories.iter().any(|memory| memory.id == rust_id));
        assert!(
            candidates
                .memories
                .iter()
                .any(|memory| memory.id == python_id)
        );
    }

    #[test]
    fn keyword_candidates_support_single_character_terms() {
        let g = MemoryGraph::in_memory().expect("graph creation failed");
        let id = g
            .store_memory("x", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");

        let candidates = keyword_candidates(g.db(), "x", 16).expect("FTS query failed");
        assert!(candidates.memories.iter().any(|memory| memory.id == id));
    }

    #[test]
    fn recall_candidate_window_keeps_access_boost_winner() {
        let g = MemoryGraph::in_memory().expect("graph creation failed");
        let winner = g
            .store_memory("needle", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");
        for _ in 0..7 {
            g.get_memory(&winner).expect("access update failed");
        }
        for i in 0..8 {
            g.store_memory(
                &format!("decoy-{i} {}", "needle ".repeat(32)),
                "",
                MemoryType::Fact,
                0.5,
                &[],
                "m",
                "",
            )
            .expect("store failed");
        }

        let results = g.recall_memories("needle", 1).expect("recall failed");
        assert_eq!(results[0].id, winner);
    }

    #[test]
    fn recall_ranks_by_keyword_hits() {
        let g = MemoryGraph::in_memory().expect("graph creation failed");
        // one keyword match
        g.store_memory("apple pie", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");
        // two keyword matches
        g.store_memory(
            "apple pie with apple sauce",
            "",
            MemoryType::Fact,
            0.5,
            &[],
            "m",
            "",
        )
        .expect("store failed");

        let results = g.recall_memories("apple", 10).expect("recall failed");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn full_scan_contract_warns_only_past_threshold() {
        assert!(full_scan_contract("memory.recall.keyword", 10, Some(5)).is_none());
        let message = full_scan_contract(
            "memory.recall.keyword",
            FULL_SCAN_WARNING_THRESHOLD,
            Some(5),
        )
        .expect("threshold row count should warn");
        assert!(message.contains("full-scan"));
        assert!(message.contains("at most 5"));
    }

    #[test]
    fn recall_respects_limit() {
        let g = MemoryGraph::in_memory().expect("graph creation failed");
        for i in 0..20 {
            g.store_memory(
                &format!("memory {i} about rust"),
                "",
                MemoryType::Fact,
                0.5,
                &[],
                "m",
                "",
            )
            .expect("store failed");
        }
        let results = g.recall_memories("rust", 5).expect("recall failed");
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn search_with_date_range() {
        let g = MemoryGraph::in_memory().expect("graph creation failed");
        g.store_memory("x", "", MemoryType::Fact, 0.5, &[], "m", "")
            .expect("store failed");

        let future = Utc::now() + chrono::Duration::days(1);
        let filter = SearchFilter {
            date_from: Some(future),
            ..Default::default()
        };
        // Nothing should be in the future.
        let results = g.search_memories(&filter).expect("search failed");
        assert!(results.is_empty());
    }
}
