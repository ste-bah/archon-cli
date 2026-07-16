use std::collections::{BTreeMap, HashMap, HashSet};

use cozo::{DataValue, DbInstance, ScriptMutability, Vector};
use ndarray::Array1;
use serde::{Deserialize, Serialize};

use crate::errors::{KnowledgeError, Result};
use crate::store::{self, DocumentChunk};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchMode {
    Exact,
    Semantic,
    Hybrid,
}

impl SearchMode {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "exact" => Ok(Self::Exact),
            "semantic" => Ok(Self::Semantic),
            "hybrid" => Ok(Self::Hybrid),
            other => Err(KnowledgeError::InvalidSearchMode(other.into())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchOptions {
    pub mode: SearchMode,
    pub top_k: usize,
    pub exact_weight: f64,
    pub semantic_weight: f64,
    pub query_embedding: Option<Vec<f32>>,
    pub document_filter: Option<Vec<String>>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            mode: SearchMode::Hybrid,
            top_k: 10,
            exact_weight: 0.55,
            semantic_weight: 0.45,
            query_embedding: None,
            document_filter: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeSearchResult {
    pub artifact_id: String,
    pub document_id: String,
    pub content: String,
    pub exact_score: f64,
    pub semantic_score: f64,
    pub combined_score: f64,
    pub source_kind: String,
}

pub fn search(
    db: &DbInstance,
    query: &str,
    options: &SearchOptions,
) -> Result<Vec<KnowledgeSearchResult>> {
    if options.top_k == 0 {
        return Ok(Vec::new());
    }
    let mut scores = match options.mode {
        SearchMode::Exact => exact_results(db, query, options)?,
        SearchMode::Semantic => semantic_results(db, options)?,
        SearchMode::Hybrid => merge_results(query, db, options)?,
    };
    scores.sort_by(|a, b| {
        b.combined_score
            .partial_cmp(&a.combined_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scores.truncate(options.top_k);
    Ok(scores)
}

fn merge_results(
    query: &str,
    db: &DbInstance,
    options: &SearchOptions,
) -> Result<Vec<KnowledgeSearchResult>> {
    let semantic = semantic_results(db, options)?;
    let mut exact_by_id: HashMap<String, KnowledgeSearchResult> =
        exact_results(db, query, options)?
            .into_iter()
            .map(|result| (result.artifact_id.clone(), result))
            .collect();
    for sem in semantic {
        if let Some(existing) = exact_by_id.get_mut(&sem.artifact_id) {
            existing.semantic_score = sem.semantic_score;
        } else {
            exact_by_id.insert(
                sem.artifact_id.clone(),
                KnowledgeSearchResult {
                    combined_score: options.semantic_weight * sem.semantic_score,
                    ..sem
                },
            );
        }
    }
    for result in exact_by_id.values_mut() {
        result.combined_score = options.exact_weight * result.exact_score
            + options.semantic_weight * result.semantic_score;
    }
    Ok(exact_by_id.into_values().collect())
}

fn exact_results(
    db: &DbInstance,
    query: &str,
    options: &SearchOptions,
) -> Result<Vec<KnowledgeSearchResult>> {
    let query_terms = tokenize(query);
    if query_terms.is_empty() {
        return Ok(Vec::new());
    }
    let fts_query = query_terms
        .iter()
        .map(|term| &term[..2])
        .collect::<Vec<_>>()
        .join(" OR ");
    let candidates = store::search_doc_chunks_fts(
        db,
        &fts_query,
        store::count_doc_chunks(db)?,
        options.document_filter.as_deref(),
    )?;
    Ok(exact_results_for_chunks(&query_terms, &candidates))
}

fn exact_results_for_chunks(
    query_terms: &[String],
    chunks: &[DocumentChunk],
) -> Vec<KnowledgeSearchResult> {
    chunks
        .iter()
        .filter_map(|chunk| {
            let score = exact_score(&query_terms, &chunk.content);
            (score > 0.0).then(|| KnowledgeSearchResult {
                artifact_id: chunk.chunk_id.clone(),
                document_id: chunk.document_id.clone(),
                content: chunk.content.clone(),
                exact_score: score,
                semantic_score: 0.0,
                combined_score: score,
                source_kind: "doc_chunk".into(),
            })
        })
        .collect()
}

fn semantic_results(
    db: &DbInstance,
    options: &SearchOptions,
) -> Result<Vec<KnowledgeSearchResult>> {
    if options.document_filter.as_ref().is_some_and(Vec::is_empty) {
        return Ok(Vec::new());
    }
    if let Some(query_embedding) = &options.query_embedding {
        let chunk_count = store::count_doc_chunks(db)?;
        if chunk_count == 0 {
            return Ok(Vec::new());
        }
        let top_k = if options.document_filter.is_some() {
            options.top_k.saturating_mul(4).min(chunk_count)
        } else {
            options.top_k
        };
        return hnsw_results(
            db,
            query_embedding,
            top_k,
            options.document_filter.as_deref(),
        );
    }
    Ok(Vec::new())
}

fn hnsw_results(
    db: &DbInstance,
    query_embedding: &[f32],
    top_k: usize,
    document_filter: Option<&[String]>,
) -> Result<Vec<KnowledgeSearchResult>> {
    let k = cozo_limit(top_k)?;
    let ef = cozo_limit(top_k.max(50))?;
    let mut params = BTreeMap::new();
    params.insert(
        "query".into(),
        DataValue::Vec(Vector::F32(Array1::from_vec(query_embedding.to_vec()))),
    );
    params.insert("k".into(), DataValue::from(k));
    params.insert("ef".into(), DataValue::from(ef));
    let filter_clause = if let Some(document_ids) = document_filter {
        params.insert(
            "document_ids".into(),
            DataValue::List(
                document_ids
                    .iter()
                    .map(|id| DataValue::from(id.as_str()))
                    .collect(),
            ),
        );
        ", document_id in $document_ids"
    } else {
        ""
    };
    let script = format!(
        r#"
            ?[chunk_id, document_id, content, distance] := ~vec_text_chunks:chunk_embedding_idx{{
                chunk_id |
                query: $query,
                k: $k,
                ef: $ef,
                bind_distance: distance
            }},
            *doc_chunks{{chunk_id, document_id, content}}{filter_clause}
        "#
    );
    match db.run_script(&script, params, ScriptMutability::Immutable) {
        Ok(result) => Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                let chunk_id = row.first().and_then(DataValue::get_str)?;
                let document_id = row.get(1).and_then(DataValue::get_str)?;
                let content = row.get(2).and_then(DataValue::get_str)?;
                let distance = row.get(3).and_then(DataValue::get_float).unwrap_or(1.0);
                let semantic_score = 1.0 - distance / 2.0;
                Some(KnowledgeSearchResult {
                    artifact_id: chunk_id.to_string(),
                    document_id: document_id.to_string(),
                    content: content.to_string(),
                    exact_score: 0.0,
                    semantic_score,
                    combined_score: semantic_score,
                    source_kind: "doc_chunk".into(),
                })
            })
            .collect()),
        Err(e) if store::relation_missing(&e.to_string()) => Ok(Vec::new()),
        Err(e) => Err(KnowledgeError::Store(format!(
            "semantic HNSW search failed: {e}"
        ))),
    }
}

fn cozo_limit(value: usize) -> Result<i64> {
    i64::try_from(value).map_err(|_| {
        KnowledgeError::InvalidSearchOptions("top_k exceeds CozoDB's signed integer range".into())
    })
}

fn exact_score(query_terms: &[String], content: &str) -> f64 {
    if query_terms.is_empty() {
        return 0.0;
    }
    let content_terms: HashSet<String> = tokenize(content).into_iter().collect();
    let hits = query_terms
        .iter()
        .filter(|term| content_terms.contains(term.as_str()))
        .count();
    hits as f64 / query_terms.len() as f64
}

fn tokenize(input: &str) -> Vec<String> {
    input
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() > 1)
        .map(|w| w.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modes() {
        assert_eq!(SearchMode::parse("exact").unwrap(), SearchMode::Exact);
        assert!(SearchMode::parse("bogus").is_err());
    }

    #[test]
    fn exact_score_matches_query_terms() {
        let score = exact_score(&tokenize("policy market"), "The policy defines the market.");
        assert_eq!(score, 1.0);
    }

    #[test]
    fn exact_score_is_zero_for_empty_query() {
        assert_eq!(exact_score(&[], "anything"), 0.0);
    }
}
