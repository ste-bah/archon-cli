//! CozoDB relation definitions for document artefacts.
//!
//! Each `:create` call is idempotent: if the relation already exists
//! (from a prior call in the same process), the error is silently ignored.
//! All other DDL errors (typos, type mismatches, syntax errors) propagate.

use anyhow::Result;
use cozo::{DbInstance, ScriptMutability};

/// Ensure all document-core relations exist. Idempotent.
pub fn ensure_doc_schema(db: &DbInstance) -> Result<()> {
    ensure_doc_sources(db)?;
    ensure_doc_ocr_runs(db)?;
    ensure_doc_artifacts(db)?;
    ensure_doc_pages(db)?;
    ensure_doc_chunks(db)?;
    ensure_doc_chunk_fts(db)?;
    ensure_doc_chunk_exact_fts(db)?;
    ensure_doc_chunk_spatial(db)?;
    ensure_doc_chunk_hashes(db)?;
    ensure_doc_locators(db)?;
    ensure_doc_image_descriptions(db)?;
    ensure_doc_pdf_metrics(db)?;
    ensure_doc_provenance_edges(db)?;
    ensure_doc_processing_jobs(db)?;
    ensure_doc_kb_memberships(db)?;
    ensure_doc_index_queue(db)?;
    ensure_doc_index_jobs(db)?;
    Ok(())
}

/// Ensure vector relations and HNSW indices exist. Idempotent.
/// `dim` is the text embedding dimension; `image_dim` is the IMAGE embedding dimension for
/// `vec_page_images` (CLIP ViT-B/32 = 512), or `None` for text-only providers (then `dim`
/// is a harmless placeholder since no image vectors are ever written).
pub fn ensure_vec_schema(db: &DbInstance, dim: usize, image_dim: Option<usize>) -> Result<()> {
    ensure_vec_text_chunks(db, dim)?;
    ensure_vec_text_embedding_cache(db, dim)?;
    ensure_vec_page_images(db, image_dim.unwrap_or(dim))?;
    Ok(())
}

/// Run a `:create` script, ignoring "already exists" errors only.
fn run_create(db: &DbInstance, script: &str) -> Result<()> {
    match crate::cozo_retry::run_script_guarded(
        db,
        script,
        Default::default(),
        ScriptMutability::Mutable,
        "schema creation",
    ) {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if crate::errors::COZO_RELATION_ALREADY_EXISTS
                .iter()
                .any(|phrase| msg.contains(phrase))
            {
                Ok(())
            } else {
                Err(anyhow::anyhow!("schema creation failed: {msg}"))
            }
        }
    }
}

fn ensure_doc_sources(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_sources {
            document_id: String =>
            source_path: String,
            media_type: String,
            content_hash: String,
            discovered_at: String,
            status: String,
        }"#,
    )
}

fn ensure_doc_ocr_runs(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_ocr_runs {
            ocr_run_id: String =>
            document_id: String,
            provider: String,
            mode: String,
            status: String,
            started_at: String,
            completed_at: String default "",
            duration_ms: Int default 0,
        }"#,
    )
}

fn ensure_doc_artifacts(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_artifacts {
            artifact_id: String =>
            document_id: String,
            artifact_type: String,
            content_hash: String,
            created_at: String,
            provenance_record_id: String default "",
        }"#,
    )
}

fn ensure_doc_pages(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_pages {
            page_id: String =>
            document_id: String,
            page_number: Int,
            text_hash: String default "",
            image_hash: String default "",
            width: Float default 0.0,
            height: Float default 0.0,
            provenance_record_id: String,
        }"#,
    )
}

fn ensure_doc_chunks(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_chunks {
            chunk_id: String =>
            document_id: String,
            artifact_id: String,
            chunk_index: Int,
            page_start: Int,
            page_end: Int,
            content: String,
            content_hash: String,
            embedding_status: String default "pending",
        }"#,
    )
}

fn ensure_doc_chunk_fts(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#"::fts create doc_chunks:chunk_content_fts {
            extractor: content,
            extract_filter: content != "",
            tokenizer: Simple,
            filters: [Lowercase, Stemmer('english'), Stopwords('en')],
        }"#,
    )
}

fn ensure_doc_chunk_exact_fts(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#"::fts create doc_chunks:chunk_exact_fts {
            extractor: content,
            extract_filter: content != "",
            tokenizer: NGram(2, 2, false),
            filters: [Lowercase],
        }"#,
    )
}

/// Per-chunk spatial provenance, keyed by `chunk_id` (verbatim-provenance spec §2).
/// Additive satellite — joined to `doc_chunks` at query time; never re-keys the vec store.
/// `super_box`/`blocks` are JSON-encoded strings (Cozo has no Json column, resolution #2).
fn ensure_doc_chunk_spatial(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_chunk_spatial {
            chunk_id: String =>
            page_num: Int,
            super_box: String,
            blocks: String,
            coord_space: String,
            spatial_hash: String,
        }"#,
    )
}

/// Per-chunk integrity hashes, keyed by `chunk_id` (verbatim-provenance spec §2).
/// `clean_sha256 == doc_chunks.content_hash` (resolution #4) so it is not duplicated here.
/// `commit_hash` binds text + spatial into the provenance chain (`chunks_root`).
fn ensure_doc_chunk_hashes(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_chunk_hashes {
            chunk_id: String =>
            raw_sha256: String,
            cleaning_version: String,
            commit_hash: String,
        }"#,
    )
}

/// Citation locators captured from running heads (Bekker numbers / page numbers),
/// ingestion-ports spec §4b. `bbox` is a JSON-encoded "[x0,y0,x1,y1]" string.
fn ensure_doc_locators(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_locators {
            locator_id: String =>
            document_id: String,
            page_num: Int,
            kind: String,
            value: String,
            bbox: String,
        }"#,
    )
}

fn ensure_doc_image_descriptions(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_image_descriptions {
            artifact_id: String =>
            document_id: String,
            page_number: Int default 0,
            provider: String,
            model: String,
            description: String,
            created_at: String,
            cost_usd: Float default 0.0,
        }"#,
    )
}

fn ensure_doc_pdf_metrics(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_pdf_metrics {
            document_id: String =>
            embedded_images_extracted: Int default 0,
            embedded_images_skipped_filter: Int default 0,
            image_ocr_runs: Int default 0,
            image_ocr_failures: Int default 0,
            image_vlm_descriptions: Int default 0,
            image_vlm_failures: Int default 0,
            pages_rendered: Int default 0,
        }"#,
    )
}

fn ensure_doc_provenance_edges(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_provenance_edges {
            edge_id: String =>
            from_artifact_id: String,
            to_artifact_id: String,
            edge_type: String,
            created_at: String,
        }"#,
    )
}

fn ensure_doc_processing_jobs(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_processing_jobs {
            job_id: String =>
            document_id: String,
            job_type: String,
            status: String,
            started_at: String,
            completed_at: String default "",
            error_message: String default "",
        }"#,
    )
}

fn ensure_doc_kb_memberships(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_kb_memberships {
            kb_id: String,
            document_id: String =>
            assigned_at: String,
        }"#,
    )
}

fn ensure_doc_index_queue(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_index_queue {
            chunk_id: String =>
            document_id: String,
            content_hash: String,
            priority: Int default 0,
            status: String default "pending",
            attempt_count: Int default 0,
            lease_owner: String default "",
            lease_expires_at: String default "",
            last_error: String default "",
            created_at: String,
            updated_at: String,
        }"#,
    )
}

fn ensure_doc_index_jobs(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create doc_index_jobs {
            job_id: String =>
            scope: String,
            document_id: String default "",
            provider: String default "",
            dimension: Int default 0,
            status: String,
            started_at: String,
            completed_at: String default "",
            leased: Int default 0,
            indexed: Int default 0,
            failed: Int default 0,
            skipped: Int default 0,
            last_error: String default "",
        }"#,
    )
}

fn ensure_vec_text_chunks(db: &DbInstance, dim: usize) -> Result<()> {
    // Create the stored relation with runtime dimension
    let create_rel = format!(
        ":create vec_text_chunks {{
            chunk_id: String
            =>
            embedding: <F32; {dim}>,
            provider: String
        }}"
    );
    run_create(db, &create_rel)?;

    // Create the HNSW index
    let create_idx = format!(
        "::hnsw create vec_text_chunks:chunk_embedding_idx {{
            dim: {dim},
            m: 50,
            dtype: F32,
            fields: [embedding],
            distance: Cosine,
            ef_construction: 200
        }}"
    );
    run_create(db, &create_idx)?;

    Ok(())
}

fn ensure_vec_text_embedding_cache(db: &DbInstance, dim: usize) -> Result<()> {
    let create_rel = format!(
        ":create vec_text_embedding_cache {{
            provider: String,
            content_hash: String
            =>
            chunk_id: String,
            embedding: <F32; {dim}>
        }}"
    );
    run_create(db, &create_rel)
}

fn ensure_vec_page_images(db: &DbInstance, dim: usize) -> Result<()> {
    // Migration: a DB created by a pre-CLIP build sized `vec_page_images` to the TEXT embedding
    // dimension. The `:create` below is a no-op when the relation already exists, so a stale
    // 768-dim relation would silently reject 512-dim CLIP image vectors forever (insert fails →
    // only a warning). If the existing relation's dim differs, drop it (and its HNSW index) so
    // it is recreated at the correct image dim. It only ever holds image vectors — none exist on
    // such old DBs — so the drop is safe; embeddings regenerate on (re-)ingest.
    if let Some(existing) = existing_vec_page_images_dim(db)
        && existing != dim
    {
        // Drop the HNSW index before the relation (a relation with a live index can't be removed).
        let _ = crate::cozo_retry::run_script_guarded(
            db,
            "::hnsw drop vec_page_images:page_image_embedding_idx",
            Default::default(),
            ScriptMutability::Mutable,
            "vec_page_images index drop",
        );
        let _ = crate::cozo_retry::run_script_guarded(
            db,
            "::remove vec_page_images",
            Default::default(),
            ScriptMutability::Mutable,
            "vec_page_images dim migration",
        );
    }

    let create_rel = format!(
        ":create vec_page_images {{
            page_id: String
            =>
            embedding: <F32; {dim}>,
            provider: String
        }}"
    );
    run_create(db, &create_rel)?;

    let create_idx = format!(
        "::hnsw create vec_page_images:page_image_embedding_idx {{
            dim: {dim},
            m: 50,
            dtype: F32,
            fields: [embedding],
            distance: Cosine,
            ef_construction: 200
        }}"
    );
    run_create(db, &create_idx)?;

    Ok(())
}

/// Best-effort read of the embedding dimension of an existing `vec_page_images` relation via
/// `::columns`. Returns `None` if the relation doesn't exist or the type can't be parsed.
fn existing_vec_page_images_dim(db: &DbInstance) -> Option<usize> {
    let result = db
        .run_script(
            "::columns vec_page_images",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .ok()?;
    for row in &result.rows {
        for cell in row {
            let Some(text) = cell.get_str() else { continue };
            // The embedding column's type renders as "<F32; N>" — take the digits after ';'.
            if text.contains("F32")
                && let Some(semi) = text.find(';')
            {
                let digits: String = text[semi + 1..]
                    .chars()
                    .filter(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(parsed) = digits.parse::<usize>() {
                    return Some(parsed);
                }
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
