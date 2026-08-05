use cozo::DbInstance;

use super::*;
use crate::ingest::ingest_file_with_policy;

fn test_db() -> DbInstance {
    let db = DbInstance::new("mem", "", "").unwrap();
    ensure_doc_schema(&db).unwrap();
    db
}

// Serial with the docs_global_state group: ingest touches the process-global OCR/VLM/embedding
// provider registries (and cozo's shared in-memory engine), so running concurrently with the
// serial PDF tests let a mock provider's output leak into this doc's chunks — a flaky content
// assertion. Serializing removes the race.
#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn reprocess_preserves_source_and_kb_membership() {
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("elliott-notes.md");
    fs::write(&path, "Wave one starts the impulse.\nWave two retraces.\n").unwrap();
    let policy = archon_policy::EffectivePolicy::default();

    let original = ingest_file_with_policy(&db, &path, &policy).await.unwrap();
    store::assign_document_to_kb(&db, "trading-elliott-wave", &original.document_id).unwrap();
    let old_chunks = store::list_chunks_for_doc(&db, &original.document_id).unwrap();
    assert!(!old_chunks.is_empty());

    let result = reprocess_document_with_policy(&db, &original.document_id, &policy)
        .await
        .unwrap();

    assert_eq!(result.ingest.document_id, original.document_id);
    assert!(!result.ingest.was_new);
    assert_eq!(result.cleared.chunks, old_chunks.len());
    let doc = store::get_doc_source(&db, &original.document_id)
        .unwrap()
        .unwrap();
    assert_eq!(doc.status, DocumentStatus::Ingested);
    let kb_docs = store::list_kb_document_ids(&db, "trading-elliott-wave").unwrap();
    assert_eq!(kb_docs, vec![original.document_id.clone()]);
    let new_chunks = store::list_chunks_for_doc(&db, &original.document_id).unwrap();
    assert!(!new_chunks.is_empty());
    assert_eq!(old_chunks[0].content, new_chunks[0].content);
}

#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn reprocess_rejects_changed_source_content() {
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.md");
    fs::write(&path, "original").unwrap();
    let policy = archon_policy::EffectivePolicy::default();
    let original = ingest_file_with_policy(&db, &path, &policy).await.unwrap();

    fs::write(&path, "changed").unwrap();
    let err = reprocess_document_with_policy(&db, &original.document_id, &policy)
        .await
        .unwrap_err();

    assert!(err.to_string().contains("source file content changed"));
    let chunks = store::list_chunks_for_doc(&db, &original.document_id).unwrap();
    assert_eq!(chunks.len(), 1);
}

/// Goharinejad regression (Tier-1a gate, 2026-08-05): a reprocess that yields a
/// degenerate document (zero chunks) must land Failed via the same admissibility
/// gate as fresh ingest — the old code flipped status to Ingested on pipeline
/// success alone, minting "Ingested" docs with no content.
#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn reprocess_zero_chunk_document_lands_failed_not_ingested() {
    use std::os::unix::fs::PermissionsExt;
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("hollow.pdf");
    fs::write(&pdf, b"%PDF hollow fixture").unwrap();
    // Mock the poppler toolchain: no text layer, no images, no renderable pages.
    for (name, body) in [
        ("pdftotext", "#!/usr/bin/env bash\nexit 0\n"),
        ("pdfimages", "#!/usr/bin/env bash\nexit 0\n"),
        ("pdftoppm", "#!/usr/bin/env bash\nexit 0\n"),
    ] {
        let p = dir.path().join(name);
        fs::write(&p, body).unwrap();
        let mut perms = fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&p, perms).unwrap();
    }
    unsafe {
        std::env::set_var("ARCHON_PDFTOTEXT_BIN", dir.path().join("pdftotext"));
        std::env::set_var("ARCHON_PDFIMAGES_BIN", dir.path().join("pdfimages"));
        std::env::set_var("ARCHON_PDFTOPPM_BIN", dir.path().join("pdftoppm"));
    }
    struct EnvGuard;
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                std::env::remove_var("ARCHON_PDFTOTEXT_BIN");
                std::env::remove_var("ARCHON_PDFIMAGES_BIN");
                std::env::remove_var("ARCHON_PDFTOPPM_BIN");
            }
        }
    }
    let _guard = EnvGuard;
    let policy = archon_policy::EffectivePolicy::default();

    // Fresh ingest of the hollow PDF already lands Failed (existing gate).
    let original = ingest_file_with_policy(&db, &pdf, &policy).await.unwrap();
    let doc = store::get_doc_source(&db, &original.document_id)
        .unwrap()
        .unwrap();
    assert_eq!(doc.status, DocumentStatus::Failed, "fresh ingest gate");

    // Reprocess must apply the SAME gate — Failed, never warn-and-Ingested.
    let result = reprocess_document_with_policy(&db, &original.document_id, &policy)
        .await
        .unwrap();
    assert!(result.ingest.pipeline_failed, "gate marks pipeline failed");
    assert!(
        result
            .ingest
            .warnings
            .iter()
            .any(|w| w.contains("ADMISSIBILITY")),
        "admissibility failure surfaced: {:?}",
        result.ingest.warnings
    );
    let doc = store::get_doc_source(&db, &original.document_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        doc.status,
        DocumentStatus::Failed,
        "zero-chunk reprocess must land Failed (Goharinejad class)"
    );
}
