use std::collections::BTreeMap;

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
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            mode: SearchMode::Hybrid,
            top_k: 10,
            exact_weight: 0.55,
            semantic_weight: 0.45,
            query_embedding: None,
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
    let chunks = store::list_doc_chunks(db)?;
    let mut scores = match options.mode {
        SearchMode::Exact => exact_results(query, &chunks),
        SearchMode::Semantic => semantic_results(db, &chunks, options)?,
        SearchMode::Hybrid => merge_results(query, db, &chunks, options)?,
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
    chunks: &[DocumentChunk],
    options: &SearchOptions,
) -> Result<Vec<KnowledgeSearchResult>> {
    let mut exact = exact_results(query, chunks);
    let semantic = semantic_results(db, chunks, options)?;
    for sem in semantic {
        if let Some(existing) = exact.iter_mut().find(|r| r.artifact_id == sem.artifact_id) {
            existing.semantic_score = sem.semantic_score;
            existing.combined_score = options.exact_weight * existing.exact_score
                + options.semantic_weight * sem.semantic_score;
        } else {
            exact.push(KnowledgeSearchResult {
                combined_score: options.semantic_weight * sem.semantic_score,
                ..sem
            });
        }
    }
    for result in &mut exact {
        result.combined_score = options.exact_weight * result.exact_score
            + options.semantic_weight * result.semantic_score;
    }
    Ok(exact)
}

fn exact_results(query: &str, chunks: &[DocumentChunk]) -> Vec<KnowledgeSearchResult> {
    let query_terms = tokenize(query);
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
    _chunks: &[DocumentChunk],
    options: &SearchOptions,
) -> Result<Vec<KnowledgeSearchResult>> {
    if let Some(query_embedding) = &options.query_embedding {
        return hnsw_results(db, query_embedding, options.top_k);
    }
    Ok(Vec::new())
}

fn hnsw_results(
    db: &DbInstance,
    query_embedding: &[f32],
    top_k: usize,
) -> Result<Vec<KnowledgeSearchResult>> {
    let mut params = BTreeMap::new();
    params.insert(
        "query".into(),
        DataValue::Vec(Vector::F32(Array1::from_vec(query_embedding.to_vec()))),
    );
    params.insert("k".into(), DataValue::from(top_k as i64));
    let result = db
        .run_script(
            r#"
            ?[chunk_id, distance] := ~vec_text_chunks:chunk_embedding_idx{
                chunk_id |
                query: $query,
                k: $k,
                ef: 50,
                bind_distance: distance
            }
            "#,
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| KnowledgeError::Store(format!("semantic HNSW search failed: {e}")))?;
    let chunks = store::list_doc_chunks(db)?;
    Ok(result
        .rows
        .iter()
        .filter_map(|row| {
            let chunk_id = row.first().and_then(DataValue::get_str)?;
            let distance = row.get(1).and_then(DataValue::get_float).unwrap_or(1.0);
            let chunk = chunks.iter().find(|c| c.chunk_id == chunk_id)?;
            Some(KnowledgeSearchResult {
                artifact_id: chunk.chunk_id.clone(),
                document_id: chunk.document_id.clone(),
                content: chunk.content.clone(),
                exact_score: 0.0,
                semantic_score: 1.0 - distance / 2.0,
                combined_score: 1.0 - distance / 2.0,
                source_kind: "doc_chunk".into(),
            })
        })
        .collect())
}

fn exact_score(query_terms: &[String], content: &str) -> f64 {
    if query_terms.is_empty() {
        return 0.0;
    }
    let content_terms = tokenize(content);
    let hits = query_terms
        .iter()
        .filter(|term| content_terms.contains(term))
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
