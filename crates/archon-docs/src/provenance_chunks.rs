//! V-1 — per-chunk integrity hashes → `chunks_root` → `extract_text_spatial` provenance record.
//!
//! Runs on ALL ingest (best-system: tamper-evidence everywhere). For each chunk we write a
//! `doc_chunk_hashes` row whose `commit_hash` binds the chunk's text and spatial fact; the
//! sorted commit hashes form `chunks_root`, which becomes the `output_hash` of an
//! `extract_text_spatial` provenance record committed into the chain (`archon-provenance`).
//! The OCR artifact's previously-empty `provenance_record_id` is pointed at that record.
//!
//! Resolution #4: Archon performs no text cleaning, so `clean_sha256 == doc_chunks.content_hash`
//! and `raw_sha256 == content_hash`; `cleaning_version = "none"`.

use std::collections::BTreeMap;

use cozo::DbInstance;

use archon_provenance::{ProvenanceRecord, chain, store as prov_store};

use crate::errors::DocsError;
use crate::hash::sha256_str;
use crate::models::{ChunkArtifact, ChunkHashes};
use crate::store;

/// Archon does no text cleaning between extraction and stored `content`.
pub const CLEANING_VERSION: &str = "none";
/// `spatial_hash` component for a chunk that has no `doc_chunk_spatial` row.
const NO_SPATIAL: &str = "";

/// Null-separated join, matching the provenance chain's field-separator convention.
fn null_join(parts: &[&str]) -> String {
    parts.join("\u{0}")
}

/// `commit_hash = sha256(chunk_id ∥ raw_sha256 ∥ clean_sha256 ∥ spatial_hash ∥ cleaning_version)`.
/// `clean_sha256` is the chunk's `content_hash` (no cleaning).
pub fn commit_hash(
    chunk_id: &str,
    raw_sha256: &str,
    content_hash: &str,
    spatial_hash: &str,
) -> String {
    sha256_str(&null_join(&[
        chunk_id,
        raw_sha256,
        content_hash,
        spatial_hash,
        CLEANING_VERSION,
    ]))
}

/// `chunks_root = sha256(sorted commit_hashes, null-separated)` — a flat root (v1).
/// Sorting makes the root order-independent so the verify-time DB join can return rows
/// in any order. Upgradeable to a Merkle root if per-chunk inclusion proofs are wanted.
pub fn chunks_root(mut commits: Vec<String>) -> String {
    commits.sort();
    let refs: Vec<&str> = commits.iter().map(String::as_str).collect();
    sha256_str(&null_join(&refs))
}

/// Write per-chunk `doc_chunk_hashes`, build the `chunks_root` provenance record, and point
/// the OCR artifact's `provenance_record_id` at it. `spatial_hash_by_chunk` carries the
/// `spatial_hash` for chunks that have a `doc_chunk_spatial` row (Marker path); chunks
/// without one commit to "no spatial". Returns the new provenance `record_id`.
pub(crate) fn persist_chunk_integrity(
    db: &DbInstance,
    ocr_artifact_id: &str,
    chunks: &[ChunkArtifact],
    spatial_hash_by_chunk: &BTreeMap<String, String>,
    ocr_engine: &str,
    source_file_sha256: &str,
    ocr_run_id: &str,
) -> Result<String, DocsError> {
    let mut commits = Vec::with_capacity(chunks.len());
    for c in chunks {
        let raw = c.content_hash.as_str(); // no cleaning → raw == content_hash
        let spatial = spatial_hash_by_chunk
            .get(&c.chunk_id)
            .map(String::as_str)
            .unwrap_or(NO_SPATIAL);
        let commit = commit_hash(&c.chunk_id, raw, &c.content_hash, spatial);
        store::insert_chunk_hashes(
            db,
            &ChunkHashes {
                chunk_id: c.chunk_id.clone(),
                raw_sha256: raw.to_string(),
                cleaning_version: CLEANING_VERSION.to_string(),
                commit_hash: commit.clone(),
            },
        )
        .map_err(|e| DocsError::Storage {
            message: e.to_string(),
        })?;
        commits.push(commit);
    }

    let root = chunks_root(commits);
    let parameters_json = serde_json::json!({
        "cleaning_version": CLEANING_VERSION,
        "ocr_engine": ocr_engine,
        "ocr_run_id": ocr_run_id,
        "chunk_count": chunks.len(),
    });
    let input_hashes = vec![source_file_sha256.to_string()];
    let parent_hashes: Vec<String> = Vec::new();
    let chain_hash = chain::chain_hash(
        &parent_hashes,
        "extract_text_spatial",
        &input_hashes,
        &root,
        Some(ocr_engine),
        None,
        &parameters_json,
    );
    let record_id = format!("prov-extract-{}", ocr_artifact_id);
    let record = ProvenanceRecord {
        record_id: record_id.clone(),
        artifact_id: ocr_artifact_id.to_string(),
        artifact_type: "ocr_text".to_string(),
        operation: "extract_text_spatial".to_string(),
        input_hashes,
        output_hash: root,
        parent_record_ids: Vec::new(),
        tool_name: Some(ocr_engine.to_string()),
        agent_name: None,
        model: None,
        parameters_json,
        timestamp: chrono::Utc::now().to_rfc3339(),
        chain_hash,
    };
    prov_store::insert_record(db, &record).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;
    store::set_artifact_provenance_record(db, ocr_artifact_id, &record_id).map_err(|e| {
        DocsError::Storage {
            message: e.to_string(),
        }
    })?;
    Ok(record_id)
}

/// Recompute `chunks_root` from the stored `doc_chunk_hashes` and compare to the record's
/// `output_hash`. Any change to a chunk's text / raw hash / spatial fact changes its
/// `commit_hash` → changes the root → returns `false` (tamper-evident).
pub fn verify_chunks_root(
    db: &DbInstance,
    document_id: &str,
    record_id: &str,
) -> Result<bool, DocsError> {
    let record = prov_store::get_record(db, record_id).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;
    let record = match record {
        Some(r) => r,
        None => return Ok(false),
    };
    let commits =
        store::get_doc_commit_hashes(db, document_id).map_err(|e| DocsError::Storage {
            message: e.to_string(),
        })?;
    Ok(chunks_root(commits) == record.output_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ArtifactRecord;

    fn test_db() -> DbInstance {
        // In-memory, matching every other test in this crate. A raw sqlite
        // DbInstance has no entry in the Cozo guard registry, so the first
        // guarded DDL fails with "database has no bound Cozo guard config" —
        // production opens sqlite through `acquire_docs_db`, which registers
        // it. It also leaked a db file into /tmp per test run.
        DbInstance::new("mem", "", Default::default()).unwrap()
    }

    fn chunk(doc: &str, i: u32, content: &str) -> ChunkArtifact {
        ChunkArtifact {
            chunk_id: format!("chunk-{doc}-{i}"),
            document_id: doc.to_string(),
            artifact_id: format!("ocr-{doc}"),
            chunk_index: i,
            page_start: 1,
            page_end: 1,
            content: content.to_string(),
            content_hash: sha256_str(content),
            embedding_status: "pending".into(),
        }
    }

    #[test]
    fn commit_hash_changes_with_inputs() {
        let a = commit_hash("c-0", "raw", "clean", "");
        assert_eq!(a.len(), 64);
        assert_ne!(a, commit_hash("c-0", "raw", "clean", "spatial"));
        assert_ne!(a, commit_hash("c-1", "raw", "clean", ""));
    }

    #[test]
    fn integrity_record_verifies_then_detects_tamper() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        // Two chunks of doc "d", plus their OCR artifact (so :update can set the record id).
        let chunks = vec![chunk("d", 0, "alpha body"), chunk("d", 1, "beta body")];
        store::insert_artifact(
            &db,
            &ArtifactRecord {
                artifact_id: "ocr-d".into(),
                document_id: "d".into(),
                artifact_type: "ocr_text".into(),
                content_hash: "fh".into(),
                created_at: "t".into(),
                provenance_record_id: String::new(),
            },
        )
        .unwrap();
        for c in &chunks {
            store::insert_chunk(&db, c).unwrap();
        }

        let rid = persist_chunk_integrity(
            &db,
            "ocr-d",
            &chunks,
            &BTreeMap::new(),
            "poppler",
            "file-hash",
            "run-1",
        )
        .unwrap();

        // The artifact's provenance_record_id is now populated (the gap is closed).
        let art = store::get_artifact(&db, "ocr-d").unwrap().unwrap();
        assert_eq!(art.provenance_record_id, rid);

        // Fresh recompute matches the committed root.
        assert!(
            verify_chunks_root(&db, "d", &rid).unwrap(),
            "untampered → verifies"
        );

        // Tamper: overwrite one chunk's commit_hash → root no longer matches.
        store::insert_chunk_hashes(
            &db,
            &ChunkHashes {
                chunk_id: "chunk-d-0".into(),
                raw_sha256: "x".into(),
                cleaning_version: CLEANING_VERSION.into(),
                commit_hash: "TAMPERED".into(),
            },
        )
        .unwrap();
        assert!(
            !verify_chunks_root(&db, "d", &rid).unwrap(),
            "tampered → fails"
        );
    }

    #[test]
    fn refold_over_superset_covers_image_chunks_and_verifies() {
        let db = test_db();
        crate::schema::ensure_doc_schema(&db).unwrap();
        let text_chunks = vec![chunk("d", 0, "alpha body"), chunk("d", 1, "beta body")];
        store::insert_artifact(
            &db,
            &ArtifactRecord {
                artifact_id: "ocr-d".into(),
                document_id: "d".into(),
                artifact_type: "ocr_text".into(),
                content_hash: "fh".into(),
                created_at: "t".into(),
                provenance_record_id: String::new(),
            },
        )
        .unwrap();
        for c in &text_chunks {
            store::insert_chunk(&db, c).unwrap();
        }

        // (1) Early text-only seal (as ingest does before image enrichment).
        let rid = persist_chunk_integrity(
            &db,
            "ocr-d",
            &text_chunks,
            &BTreeMap::new(),
            "poppler",
            "file-hash",
            "run-1",
        )
        .unwrap();
        let text_only_root = chunks_root(store::get_doc_commit_hashes(&db, "d").unwrap());
        assert!(verify_chunks_root(&db, "d", &rid).unwrap());

        // (2) An image-OCR chunk lands AFTER the seal (distinct, uuid-keyed id).
        let img = ChunkArtifact {
            chunk_id: "chunk-pdf-image-ocr-xyz-0".into(),
            document_id: "d".into(),
            artifact_id: "pdf-image-ocr-xyz".into(),
            chunk_index: 0,
            page_start: 2,
            page_end: 2,
            content: "figure caption text".into(),
            content_hash: sha256_str("figure caption text"),
            embedding_status: "pending".into(),
        };
        store::insert_chunk(&db, &img).unwrap();

        // (3) Re-fold over the text+image UNION → same record, superset root.
        let mut all = text_chunks.clone();
        all.push(img);
        let rid2 = persist_chunk_integrity(
            &db,
            "ocr-d",
            &all,
            &BTreeMap::new(),
            "poppler",
            "file-hash",
            "run-1",
        )
        .unwrap();
        assert_eq!(rid2, rid, "re-fold overwrites the same provenance record");
        let superset_root = chunks_root(store::get_doc_commit_hashes(&db, "d").unwrap());
        assert_ne!(
            superset_root, text_only_root,
            "the image chunk changes the root"
        );
        assert!(
            verify_chunks_root(&db, "d", &rid).unwrap(),
            "superset root verifies"
        );

        // (4) Tampering the IMAGE chunk's commit now flips verify → image content is covered.
        store::insert_chunk_hashes(
            &db,
            &ChunkHashes {
                chunk_id: "chunk-pdf-image-ocr-xyz-0".into(),
                raw_sha256: "x".into(),
                cleaning_version: CLEANING_VERSION.into(),
                commit_hash: "TAMPERED".into(),
            },
        )
        .unwrap();
        assert!(
            !verify_chunks_root(&db, "d", &rid).unwrap(),
            "tampered image chunk → fails (was silently excluded before)"
        );
    }
}
