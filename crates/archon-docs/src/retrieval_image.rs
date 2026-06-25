//! Cross-modal image retrieval (I2-search): embed a TEXT query with the CLIP text encoder
//! and search the CLIP image vectors in `vec_page_images` (shared 512-dim space). Lets you
//! find images/frames by a text description ("the scene where …").
//!
//! Only works when the embedding provider is multimodal (`embed_image_query` returns `Some`);
//! text-only providers (e.g. plain BGE) return an explicit error rather than empty results.

use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability, Vector};

use crate::embed::{get_provider, init_default_provider};
use crate::errors::DocsError;
use crate::store;

/// One image search hit, resolved to its document + source file.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageSearchResult {
    pub page_id: String,
    pub document_id: String,
    pub source_path: String,
    pub page_number: u32,
    pub distance: f64,
    pub score: f64,
}

fn get_or_init_provider() -> Option<std::sync::Arc<dyn crate::embed::LocalEmbeddingProvider>> {
    if get_provider().is_none()
        && let Err(error) = init_default_provider()
    {
        tracing::warn!(%error, "docs embedding provider not available");
    }
    get_provider()
}

/// Find images whose CLIP visual embedding is closest to the CLIP text embedding of `query`.
pub fn search_images(
    db: &DbInstance,
    query: &str,
    top_k: usize,
) -> Result<Vec<ImageSearchResult>, DocsError> {
    let provider = get_or_init_provider().ok_or_else(|| DocsError::ModelNotConfigured {
        message: "no embedding provider configured. Run 'archon docs model-status' for details."
            .into(),
    })?;
    let query_vec = provider.embed_image_query(query)?.ok_or_else(|| DocsError::Validation {
        message: "the configured embedding provider is not multimodal — it produces no image \
                  embeddings, so text→image search is unavailable. Use a CLIP-capable provider."
            .into(),
    })?;

    let arr = ndarray::Array1::from_vec(query_vec);
    let mut params = BTreeMap::new();
    params.insert("query".to_string(), DataValue::Vec(Vector::F32(arr)));
    params.insert("k".to_string(), DataValue::from(top_k as i64));
    let script = "?[page_id, distance] := ~vec_page_images:page_image_embedding_idx{
            page_id,
            |
            query: $query,
            k: $k,
            ef: 50,
            bind_distance: distance
        }";
    let result = db
        .run_script(script, params, ScriptMutability::Immutable)
        .map_err(|e| DocsError::Retrieval {
            message: format!("image HNSW search failed: {e}"),
        })?;

    let mut results = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        let page_id = row[0].get_str().unwrap_or("").to_string();
        let distance = row[1].get_float().unwrap_or(1.0);
        let (document_id, page_number) = resolve_page(db, &page_id);
        let source_path = store::get_doc_source(db, &document_id)
            .ok()
            .flatten()
            .map(|d| d.source_path)
            .unwrap_or_default();
        results.push(ImageSearchResult {
            page_id,
            document_id,
            source_path,
            page_number,
            distance,
            score: 1.0 - distance / 2.0,
        });
    }
    results.sort_by(|a, b| {
        a.distance
            .partial_cmp(&b.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(results)
}

/// Resolve a `page_id` to its `(document_id, page_number)` via `doc_pages`. PDF figure
/// embeddings use a `"{page_id}-img{N}"` key (multiple figures per page); strip that suffix
/// back to the real page before lookup so figure hits still resolve to their PDF + page.
fn resolve_page(db: &DbInstance, page_id: &str) -> (String, u32) {
    let lookup = match page_id.rsplit_once("-img") {
        Some((base, n)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => base,
        _ => page_id,
    };
    let mut params = BTreeMap::new();
    params.insert("pid".to_string(), DataValue::from(lookup));
    let script = "?[document_id, page_number] := \
                  *doc_pages{page_id, document_id, page_number}, page_id = $pid";
    match db.run_script(script, params, ScriptMutability::Immutable) {
        Ok(r) => r
            .rows
            .first()
            .map(|row| {
                (
                    row[0].get_str().unwrap_or("").to_string(),
                    row[1].get_int().unwrap_or(0) as u32,
                )
            })
            .unwrap_or_default(),
        Err(_) => (String::new(), 0),
    }
}
