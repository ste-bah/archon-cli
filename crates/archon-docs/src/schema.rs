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
    ensure_doc_provenance_edges(db)?;
    ensure_doc_processing_jobs(db)?;
    Ok(())
}

/// Ensure vector relations and HNSW indices exist. Idempotent.
/// `dim` must be the dimension from the embedding provider.
pub fn ensure_vec_schema(db: &DbInstance, dim: usize) -> Result<()> {
    ensure_vec_text_chunks(db, dim)?;
    ensure_vec_page_images(db, dim)?;
    Ok(())
}

/// Run a `:create` script, ignoring "already exists" errors only.
fn run_create(db: &DbInstance, script: &str) -> Result<()> {
    match db.run_script(script, Default::default(), ScriptMutability::Mutable) {
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

fn ensure_vec_page_images(db: &DbInstance, dim: usize) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        DbInstance::new("mem", "", Default::default()).unwrap()
    }

    #[test]
    fn test_ensure_schema_idempotent() {
        let db = test_db();
        ensure_doc_schema(&db).unwrap();
        ensure_doc_schema(&db).unwrap(); // second call must not panic
    }

    #[test]
    fn test_run_create_propagates_real_errors() {
        let db = test_db();
        // Syntax error should propagate, not be silently swallowed
        let result = run_create(&db, ":create bad_syntax {");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(!msg.contains("conflicts with an existing"));
        assert!(!msg.contains("already exists"));
    }

    #[test]
    fn test_run_create_ignores_both_already_exists_phrasings() {
        let db = test_db();
        // Create once — succeeds
        run_create(&db, ":create phrasing_test { id: String => val: String }").unwrap();
        // Create again — fails with an "already exists" error, must be ignored
        run_create(&db, ":create phrasing_test { id: String => val: String }").unwrap();
        // If we got here, the error was correctly suppressed
    }

    #[test]
    fn test_vec_schema_idempotent() {
        let db = test_db();
        ensure_doc_schema(&db).unwrap();
        ensure_vec_schema(&db, 768).unwrap();
        // Second call must be silent
        ensure_vec_schema(&db, 768).unwrap();
    }

    #[test]
    fn test_ensure_vec_schema_creates_relation_lazily() {
        let db = test_db();
        // Fresh DB: only doc schema (NOT vec schema)
        ensure_doc_schema(&db).unwrap();

        // Inserting a vector must fail because vec_text_chunks doesn't exist
        let mut params = std::collections::BTreeMap::new();
        let v = ndarray::Array1::from_vec(vec![0.0_f32; 768]);
        params.insert("cid".to_string(), cozo::DataValue::from("test-chunk"));
        params.insert(
            "emb".to_string(),
            cozo::DataValue::Vec(cozo::Vector::F32(v)),
        );
        params.insert("prov".to_string(), cozo::DataValue::from("test"));
        let before = db.run_script(
            "?[chunk_id, embedding, provider] <- [[$cid, $emb, $prov]]
             :put vec_text_chunks { chunk_id => embedding, provider }",
            params.clone(),
            cozo::ScriptMutability::Mutable,
        );
        assert!(
            before.is_err(),
            "vector insert must fail before ensure_vec_schema"
        );

        // Now create vec schema
        ensure_vec_schema(&db, 768).unwrap();

        // Same insert must succeed after ensure_vec_schema
        let after = db.run_script(
            "?[chunk_id, embedding, provider] <- [[$cid, $emb, $prov]]
             :put vec_text_chunks { chunk_id => embedding, provider }",
            params,
            cozo::ScriptMutability::Mutable,
        );
        assert!(
            after.is_ok(),
            "vector insert must succeed after ensure_vec_schema"
        );
    }

    #[test]
    fn test_vec_schema_rejects_wrong_dimension() {
        let db = test_db();
        ensure_doc_schema(&db).unwrap();
        ensure_vec_schema(&db, 768).unwrap();
        // Insert with wrong dimension must fail
        let mut params = std::collections::BTreeMap::new();
        let wrong_vec = ndarray::Array1::from_vec(vec![0.0_f32; 384]);
        params.insert("cid".to_string(), cozo::DataValue::from("test-chunk"));
        params.insert(
            "emb".to_string(),
            cozo::DataValue::Vec(cozo::Vector::F32(wrong_vec)),
        );
        params.insert("prov".to_string(), cozo::DataValue::from("test"));
        let result = db.run_script(
            "?[chunk_id, embedding, provider] <- [[$cid, $emb, $prov]]
             :put vec_text_chunks { chunk_id => embedding, provider }",
            params,
            cozo::ScriptMutability::Mutable,
        );
        assert!(result.is_err(), "wrong-dimension vector insert must fail");
    }

    #[test]
    fn test_cozo_relation_not_found_marker() {
        let db = test_db();
        // Query a relation that doesn't exist — Cozo must return an error
        // containing COZO_RELATION_NOT_FOUND. If this fails, the Cozo version
        // changed its error phrasing and the constant needs updating.
        let result = db.run_script(
            "?[chunk_id] := *nonexistent_relation_xyz{chunk_id}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        );
        assert!(result.is_err(), "querying nonexistent relation must fail");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains(crate::errors::COZO_RELATION_NOT_FOUND),
            "Cozo error must contain COZO_RELATION_NOT_FOUND marker.\n\
             Expected to contain: {}\n\
             Actual error: {msg}",
            crate::errors::COZO_RELATION_NOT_FOUND,
        );
    }
}
