use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::graph::{raw_to_memory, read_all_memories, row_values_to_memory};
use crate::types::{Memory, MemoryError, SearchFilter};

pub(crate) const FULL_SCAN_WARNING_THRESHOLD: usize = 10_000;

/// Maximum terms OR'd into one FTS query.
///
/// Callers hand us whole prompts. A slash-command turn arrives carrying its
/// entire skill template -- 21 KB, ~3,200 words, ~1,500 distinct terms after
/// stop-word removal -- and every one of those became an `OR` branch. Cozo
/// evaluates every branch of an `Or` in full before merging, and with an
/// `NGram` tokenizer each branch expands further into character n-grams whose
/// posting lists are close to the whole relation. The result was minutes of
/// single-threaded CPU per turn.
///
/// A recall query does not get more precise past a few dozen terms; it just
/// gets slower, because each additional term can only widen an `OR`. Longest
/// terms are kept as a cheap proxy for selectivity -- absent term frequencies
/// here, length is the best available signal, and short common words are
/// exactly what a bounded query should shed first.
const MAX_FTS_TERMS: usize = 32;

/// Shortest term worth querying. One- and two-character tokens match nearly
/// everything and contribute noise rather than recall.
const MIN_FTS_TERM_LEN: usize = 3;

/// Hard ceiling on candidates requested from one FTS index.
///
/// Cozo pre-allocates for `k`, so an unbounded value is not merely slow -- it
/// aborts on a capacity overflow inside `raw_vec`. `structured_search_candidates`
/// genuinely passes `usize::MAX` (there is no limit field on `SearchFilter` to
/// pass instead), which the previous doubling loop masked by starting at 256
/// and growing. Requesting the caller's limit directly exposed it.
///
/// Well above any real recall window, so it changes no result a caller could
/// observe; it exists so an unbounded request degrades to "a lot" instead of a
/// crash.
const MAX_FTS_CANDIDATES: usize = 4_096;

/// Minimum candidates fetched per index before re-ranking.
///
/// Candidates are not results. `recall_memories` re-ranks FTS hits by access
/// boost and vector similarity, so a memory that TF-IDF ranks poorly can still
/// win — `recall_ranks_access_boost_winner_beyond_fts_window` pins exactly
/// that, with one frequently-accessed memory behind 300 higher-frequency
/// decoys and a caller limit of 1. Fetch only `limit` candidates and such a
/// memory is never in the set to be re-ranked, so the boost can never apply.
/// A floor is therefore required, not optional.
///
/// SIZED, not rounded. The previous value was 1,024 — chosen because it was the
/// next round number above the 301 rows that test needs. Every fetched row is
/// deserialised by `row_values_to_memory`, three indexes deep, so that figure
/// cost ~3,000 row decodes on a request for 16 candidates. 384 clears the
/// pinning case with headroom and cuts the decode work to under a third.
///
/// If a future test needs a wider window, raise this and say why here rather
/// than picking another round number.
const RECALL_CANDIDATE_WINDOW: usize = 384;
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

    match fts_keyword_candidates(db, query, limit) {
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

/// Ask each index once, for the number of candidates the caller actually wants.
///
/// This previously started at 256 and DOUBLED the limit, re-running the whole
/// query, until a page came back short -- which only happens once the limit
/// exceeds the total number of matching rows. So it deliberately retrieved
/// every match, and because Cozo recomputes the full result set on each call
/// (its `k` truncates the finished set rather than pruning the search), the
/// same exhaustive scan ran ~log2(N/256) times over, per index, three indexes
/// deep. The caller's `limit` was ignored entirely on this path.
///
/// One query per index. `k` is a candidate WINDOW, not the caller's limit:
/// `recall_memories` re-ranks what comes back, so a memory the FTS scores
/// poorly can still win on access boost and must be in the set to do so. See
/// [`RECALL_CANDIDATE_WINDOW`] for why a floor is required and how it is sized.
///
/// The window is deliberately bounded at both ends. `clamp` here reads as a
/// floor as much as a ceiling, and that is intended -- a caller asking for 16
/// still fetches [`RECALL_CANDIDATE_WINDOW`] candidates, because 16 is a result
/// count and this is a candidate count.
fn fts_keyword_candidates(
    db: &DbInstance,
    query: &str,
    limit: usize,
) -> Result<Vec<Memory>, MemoryError> {
    let mut memories = Vec::new();
    let mut seen = BTreeSet::new();
    let window = limit.clamp(RECALL_CANDIDATE_WINDOW, MAX_FTS_CANDIDATES);
    let limit = i64::try_from(window).unwrap_or(i64::MAX);

    for index in ["content_fts", "title_fts", "tags_fts"] {
        let result = fts_candidate_query(db, query, index, limit)?;
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

/// Build a bounded `OR` query from the caller's text.
///
/// Deduplicates case-insensitively, drops tokens below [`MIN_FTS_TERM_LEN`],
/// and keeps at most [`MAX_FTS_TERMS`] of the longest remaining terms. Original
/// spelling and relative order are preserved for the terms that survive, so the
/// query still reads as a subset of what the caller asked for.
fn fts_query(query: &str) -> String {
    let mut seen = BTreeSet::new();
    let mut terms: Vec<&str> = query
        .split_whitespace()
        .filter(|term| seen.insert(term.to_lowercase()))
        .collect();

    // Short tokens are dropped only from an OVERSIZED query, never from a small
    // one. `keyword_candidates_support_single_character_terms` pins that a
    // deliberate one-character search still works, and it should: the cost of a
    // low-selectivity term is a problem of volume, not of length. Filtering
    // unconditionally would have silently broken a supported query shape while
    // "fixing" performance.
    if terms.len() > MAX_FTS_TERMS {
        terms.retain(|term| term.chars().count() >= MIN_FTS_TERM_LEN);
    }

    if terms.len() > MAX_FTS_TERMS {
        // Longest-first as a selectivity proxy, then restore input order so the
        // query is stable and readable in a log.
        let mut ranked: Vec<(usize, &str)> = terms.iter().copied().enumerate().collect();
        ranked.sort_by_key(|(_, term)| std::cmp::Reverse(term.chars().count()));
        ranked.truncate(MAX_FTS_TERMS);
        ranked.sort_by_key(|(index, _)| *index);
        terms = ranked.into_iter().map(|(_, term)| term).collect();
    }

    terms
        .into_iter()
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
    // Push the caller's bound into the query rather than reading everything and
    // trimming afterwards. `SearchFilter::candidate_limit()` still yields
    // `usize::MAX` for `None`, so a caller that never sets a limit keeps its
    // historical unbounded contract -- and `fts_keyword_candidates` clamps that
    // to `MAX_FTS_CANDIDATES` so it can no longer overflow Cozo's `k`
    // pre-allocation.
    if let Some(text) = filter.text.as_deref() {
        return keyword_candidates(db, text, filter.candidate_limit());
    }
    if !filter.tags.is_empty() {
        return keyword_candidates(db, &filter.tags.join(" "), filter.candidate_limit());
    }
    // No text and no tags: there is no FTS query to bound, so this still reads
    // the relation in full. The limit is reported in the warning rather than
    // applied, because trimming here would cut the set before the caller's own
    // filtering and ordering run.
    let all_rows = read_all_memories(db)?;
    warn_full_scan("memory.search.filter", all_rows.len(), filter.limit);
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
