use chrono::Utc;
use cozo::DbInstance;

use crate::graph::{raw_to_memory, read_all_memories};
use crate::types::{Memory, MemoryError, SearchFilter};

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

    // Fetch all memories and score them in Rust.
    let all_rows = read_all_memories(db)?;
    let now = Utc::now();

    let mut scored: Vec<(f64, Memory)> = Vec::new();

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

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(limit);

    Ok(scored.into_iter().map(|(_, m)| m).collect())
}

/// Structured search with filters (type, tags, text, date range).
pub(crate) fn search(
    db: &DbInstance,
    filter: &SearchFilter,
) -> Result<Vec<Memory>, MemoryError> {
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

    let mut results: Vec<Memory> = Vec::new();

    for raw in all_rows {
        let mem = match raw_to_memory(raw) {
            Ok(m) => m,
            Err(_) => continue,
        };

        // Filter by memory_type
        if let Some(mt) = filter.memory_type {
            if mem.memory_type != mt {
                continue;
            }
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
        if let Some(ref from) = filter.date_from {
            if mem.created_at < *from {
                continue;
            }
        }

        // Filter by date_to
        if let Some(ref to) = filter.date_to {
            if mem.created_at > *to {
                continue;
            }
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
    results.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::MemoryGraph;
    use crate::types::MemoryType;

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
