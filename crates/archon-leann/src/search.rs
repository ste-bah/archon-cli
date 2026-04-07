//! Natural language and code similarity search via CozoDB HNSW.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability, Vector};
use ndarray::Array1;

use archon_memory::embedding::EmbeddingProvider;

use crate::metadata::SearchResult;

/// Default minimum similarity threshold (matching TypeScript default of 0.3).
pub const DEFAULT_MIN_SIMILARITY: f64 = 0.3;

/// Default max results.
pub const DEFAULT_MAX_RESULTS: usize = 5;

/// Over-fetch multiplier for filter headroom.
const OVERFETCH_MULTIPLIER: usize = 3;

/// Natural language and code similarity search using CozoDB HNSW index.
pub struct Search {
    db: DbInstance,
    embedder: Arc<dyn EmbeddingProvider>,
}

impl Search {
    /// Create a new `Search` from an existing CozoDB instance and embedding provider.
    pub fn new(db: DbInstance, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self { db, embedder }
    }

    /// Embed query, run CozoDB HNSW search, filter by threshold, sort descending.
    pub fn search_code(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        self.search_with_filter(query, limit, None, None)
    }

    /// Embed code snippet, run HNSW search, dedup by file_path (highest score wins).
    pub fn find_similar_code(&self, code: &str, limit: usize) -> Result<Vec<SearchResult>> {
        // Overfetch to have headroom for dedup
        let fetch_k = (limit * OVERFETCH_MULTIPLIER).max(limit + 10);

        let query_vec = self.embed_text(code)?;
        let raw = self.run_hnsw_query(&query_vec, fetch_k)?;

        // Apply similarity threshold
        let mut filtered: Vec<SearchResult> = raw
            .into_iter()
            .filter(|r| r.relevance_score >= DEFAULT_MIN_SIMILARITY)
            .collect();

        // Dedup: group by file_path, keep highest relevance_score per file
        let mut best_per_file: HashMap<PathBuf, SearchResult> = HashMap::new();
        for result in filtered.drain(..) {
            let entry = best_per_file.entry(result.file_path.clone());
            match entry {
                std::collections::hash_map::Entry::Occupied(mut occ) => {
                    if result.relevance_score > occ.get().relevance_score {
                        occ.insert(result);
                    }
                }
                std::collections::hash_map::Entry::Vacant(vac) => {
                    vac.insert(result);
                }
            }
        }

        // Collect, sort descending, take limit
        let mut results: Vec<SearchResult> = best_per_file.into_values().collect();
        results.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        Ok(results)
    }

    /// Search with optional language and path_pattern filters (post-retrieval).
    pub fn search_with_filter(
        &self,
        query: &str,
        limit: usize,
        language: Option<&str>,
        path_pattern: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        // Overfetch to have headroom after filtering
        let fetch_k = (limit * OVERFETCH_MULTIPLIER).max(limit + 10);

        let query_vec = self.embed_text(query)?;
        let raw = self.run_hnsw_query(&query_vec, fetch_k)?;

        // Post-filter: language
        let after_lang: Vec<SearchResult> = if let Some(lang) = language {
            raw.into_iter().filter(|r| r.language == lang).collect()
        } else {
            raw
        };

        // Post-filter: path_pattern (substring match)
        let after_path: Vec<SearchResult> = if let Some(pattern) = path_pattern {
            after_lang
                .into_iter()
                .filter(|r| r.file_path.to_string_lossy().contains(pattern))
                .collect()
        } else {
            after_lang
        };

        // Filter below threshold
        let mut above_threshold: Vec<SearchResult> = after_path
            .into_iter()
            .filter(|r| r.relevance_score >= DEFAULT_MIN_SIMILARITY)
            .collect();

        // Sort descending
        above_threshold.sort_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Take limit
        above_threshold.truncate(limit);

        Ok(above_threshold)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Embed a single text string and return the embedding vector.
    fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        let embeddings = self
            .embedder
            .embed(&[text.to_string()])
            .map_err(|e| anyhow::anyhow!("embedding failed: {}", e))?;

        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("embedder returned no vectors"))
    }

    /// Execute an HNSW similarity query against CozoDB and return raw results.
    ///
    /// Distance from CozoDB Cosine HNSW is in [0, 2] (0 = identical, 2 = opposite).
    /// Converts to similarity: `relevance_score = 1.0 - (distance / 2.0)`.
    fn run_hnsw_query(&self, embedding: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        let arr = Array1::from_vec(embedding.to_vec());
        let query_vec = DataValue::Vec(Vector::F32(arr));

        let mut params = BTreeMap::new();
        params.insert("query_vec".to_string(), query_vec);
        params.insert("k".to_string(), DataValue::from(k as i64));

        let script = r#"
?[chunk_id, file_path, language, line_start, line_end, chunk_content, distance] :=
    ~code_chunks:chunk_embedding_idx{
        chunk_id,
        file_path,
        language,
        line_start,
        line_end,
        chunk_content
        |
        query: $query_vec,
        k: $k,
        ef: 200,
        bind_distance: distance
    }
"#;

        let result = self
            .db
            .run_script(script, params, ScriptMutability::Immutable)
            .map_err(|e| anyhow::anyhow!("HNSW search failed: {}", e))?;

        let mut search_results = Vec::with_capacity(result.rows.len());

        for row in result.rows {
            // Expected columns: chunk_id, file_path, language, line_start, line_end, chunk_content, distance
            if row.len() < 7 {
                continue;
            }

            let file_path = match row[1].get_str() {
                Some(s) => PathBuf::from(s),
                None => continue,
            };
            let language = match row[2].get_str() {
                Some(s) => s.to_string(),
                None => continue,
            };
            let line_start = match row[3].get_int() {
                Some(n) => n as usize,
                None => continue,
            };
            let line_end = match row[4].get_int() {
                Some(n) => n as usize,
                None => continue,
            };
            let content = match row[5].get_str() {
                Some(s) => s.to_string(),
                None => continue,
            };
            let distance: f64 = match &row[6] {
                DataValue::Num(cozo::Num::Float(f)) => *f,
                DataValue::Num(cozo::Num::Int(i)) => *i as f64,
                _ => continue,
            };

            // Convert cosine distance [0, 2] to similarity [0.0, 1.0]
            let relevance_score = (1.0_f64 - distance / 2.0_f64).clamp(0.0_f64, 1.0_f64);

            search_results.push(SearchResult {
                file_path,
                content,
                language,
                line_start,
                line_end,
                relevance_score,
            });
        }

        Ok(search_results)
    }
}
