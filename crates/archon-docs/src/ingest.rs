//! Document ingestion engine.
//!
//! File-type detection, direct text/Markdown ingest, directory walking,
//! duplicate detection by content hash, and the full OCR → chunk →
//! page → provenance pipeline (Phase 1).
//!
//! Implements REQ-DOCS-001, REQ-DOCS-002, REQ-DOCS-004.

use std::fs;
use std::path::Path;

use anyhow::Result;
use cozo::DbInstance;
use tracing::info;

use crate::chunking::{build_chunk_artifacts, chunk_with_page_anchors};
use crate::errors::DocsError;
use crate::hash::{sha256_hex, sha256_str};
use crate::models::{
    ArtifactRecord, DocumentStatus, OcrRun, OcrStatus, PageArtifact, ProcessingJob, SourceDocument,
};
use crate::ocr::local::LocalOcrProvider;
use crate::ocr::provider::{OcrProvider, OcrRequest};
use crate::provenance::build_doc_lineage_edges;
use crate::schema::ensure_doc_schema;
use crate::store::{self, hash_exists_in_sources};

use crate::embed;
use crate::retrieval;

/// Result of a single-file ingest.
#[derive(Clone, Debug)]
pub struct IngestFileResult {
    pub document_id: String,
    pub was_new: bool,
    pub ocr_skipped: bool,
}

/// Result of a directory ingest operation.
#[derive(Clone, Debug, Default)]
pub struct IngestResult {
    pub sources_registered: usize,
    pub sources_skipped_duplicate: usize,
    pub sources_failed: usize,
    pub images_skipped: usize,
    pub errors: Vec<String>,
}

/// Detect media type from file extension.
pub fn detect_media_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "tiff" | "tif" => "image/tiff",
        "md" | "markdown" => "text/markdown",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Determine whether a media type is supported for Phase 1 ingest.
pub fn is_supported_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "text/plain"
            | "text/markdown"
            | "application/pdf"
            | "image/png"
            | "image/jpeg"
            | "image/tiff"
    )
}

/// Determine whether the media type can go through the OCR → chunk pipeline.
fn is_ocr_runnable(media_type: &str) -> bool {
    matches!(media_type, "text/plain" | "text/markdown" | "application/pdf")
}

/// Ingest a single file: register document source, run OCR + chunk +
/// page + provenance pipeline, update status to Ingested.
pub async fn ingest_file(
    db: &DbInstance,
    path: &Path,
) -> Result<IngestFileResult, DocsError> {
    ensure_doc_schema(db).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    let source_path = path.to_string_lossy().to_string();
    let media_type = detect_media_type(path);

    if !is_supported_media_type(media_type) {
        return Err(DocsError::UnsupportedMediaType {
            media_type: media_type.to_string(),
        });
    }

    // Read and hash file content
    let content_bytes =
        fs::read(path).map_err(|e| DocsError::OcrFile {
            path: source_path.clone(),
            message: e.to_string(),
        })?;

    let content_hash = sha256_hex(&content_bytes);

    // Check for duplicate by content hash
    if hash_exists_in_sources(db, &content_hash).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })? {
        // Return existing document_id so callers can link to it
        let existing_id = store::get_doc_by_hash(db, &content_hash)
            .map_err(|e| DocsError::Storage { message: e.to_string() })?
            .map(|d| d.document_id)
            .unwrap_or_default();
        info!(
            path = %source_path,
            hash = %content_hash,
            existing = %existing_id,
            "Skipping duplicate document"
        );
        return Ok(IngestFileResult {
            document_id: existing_id,
            was_new: false,
            ocr_skipped: false,
        });
    }

    // Register the source document
    let document_id = format!("doc-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();

    let doc = SourceDocument {
        document_id: document_id.clone(),
        source_path: source_path.clone(),
        media_type: media_type.to_string(),
        content_hash: content_hash.clone(),
        discovered_at: now.clone(),
        status: DocumentStatus::Discovered,
    };

    store::insert_doc_source(db, &doc).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    // Create processing job
    let job = ProcessingJob {
        job_id: format!("job-{}", uuid::Uuid::new_v4()),
        document_id: document_id.clone(),
        job_type: "ingest".to_string(),
        status: "pending".to_string(),
        started_at: now,
        completed_at: None,
        error_message: None,
    };
    store::insert_processing_job(db, &job).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    // ── OCR → chunk → page → provenance pipeline ──────────────────
    let mut ocr_skipped = false;
    if is_ocr_runnable(media_type) {
        // Transition to Ingesting before starting the pipeline
        store::update_doc_status(db, &document_id, &DocumentStatus::Ingesting)
            .map_err(|e| DocsError::Storage { message: e.to_string() })?;

        match run_ingest_pipeline(db, &document_id, &source_path, media_type).await {
            Ok(()) => {
                store::update_doc_status(db, &document_id, &DocumentStatus::Ingested)
                    .map_err(|e| DocsError::Storage { message: e.to_string() })?;
            }
            Err(e) => {
                store::update_doc_status(db, &document_id, &DocumentStatus::Failed)
                    .map_err(|e| DocsError::Storage { message: e.to_string() })?;
                info!(
                    document_id = %document_id,
                    error = %e,
                    "OCR pipeline failed; document set to Failed"
                );
                // Don't fail the ingest — document is registered, pipeline can be retried
            }
        }
    } else {
        // Image files: registered but no OCR extraction in Phase 1
        ocr_skipped = true;
        tracing::warn!(
            document_id = %document_id,
            path = %source_path,
            media_type = %media_type,
            "Image OCR not yet implemented in Phase 1; registered without text extraction"
        );
        store::update_doc_status(db, &document_id, &DocumentStatus::Ingested)
            .map_err(|e| DocsError::Storage { message: e.to_string() })?;
    }

    Ok(IngestFileResult {
        document_id,
        was_new: true,
        ocr_skipped,
    })
}

/// Run the full OCR → page records → chunking → provenance pipeline
/// for a single document. All inserts go through the Cozo `db`.
async fn run_ingest_pipeline(
    db: &DbInstance,
    document_id: &str,
    file_path: &str,
    media_type: &str,
) -> Result<(), DocsError> {
    let provider = LocalOcrProvider;
    let ocr_run_id = format!("ocr-{}", uuid::Uuid::new_v4());

    // 1. Create OCR run (Running)
    let ocr_run = OcrRun {
        ocr_run_id: ocr_run_id.clone(),
        document_id: document_id.to_string(),
        provider: provider.name().to_string(),
        mode: media_type.to_string(),
        status: OcrStatus::Running,
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        duration_ms: None,
    };
    store::insert_ocr_run(db, &ocr_run).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    // 2. Run OCR extraction
    let request = OcrRequest {
        file_path: file_path.to_string(),
        document_id: document_id.to_string(),
        ocr_run_id: ocr_run_id.clone(),
        page_range: None,
        language_hint: None,
    };

    let extract_result = match provider.extract(request).await {
        Ok(result) => result,
        Err(e) => {
            // Mark OCR run as Failed before propagating
            let _ = store::update_ocr_run_completion(
                db,
                &ocr_run_id,
                &OcrStatus::Failed,
                &chrono::Utc::now().to_rfc3339(),
                0,
            );
            return Err(e);
        }
    };

    // Update OCR run to Completed
    let completed_at = chrono::Utc::now().to_rfc3339();
    store::update_ocr_run_completion(
        db,
        &ocr_run_id,
        &OcrStatus::Completed,
        &completed_at,
        extract_result.processing_duration_ms,
    )
    .map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    // 3. Create page records from page_offsets
    let mut page_ids: Vec<String> = Vec::new();
    for po in &extract_result.page_offsets {
        let page_id = format!("page-{}-{}", document_id, po.page);
        let page_text = &extract_result.full_text[po.char_start..po.char_end];
        let page = PageArtifact {
            page_id: page_id.clone(),
            document_id: document_id.to_string(),
            page_number: po.page,
            text_hash: Some(sha256_str(page_text)),
            image_hash: None,
            width: None,
            height: None,
            provenance_record_id: String::new(),
        };
        store::insert_page(db, &page).map_err(|e| DocsError::Storage {
            message: e.to_string(),
        })?;
        page_ids.push(page_id);
    }

    // 4. Create OCR-result artifact for provenance chain
    let ocr_artifact_id = format!("ocr-result-{}", ocr_run_id);
    let ocr_artifact = ArtifactRecord {
        artifact_id: ocr_artifact_id.clone(),
        document_id: document_id.to_string(),
        artifact_type: "ocr_text".to_string(),
        content_hash: sha256_str(&extract_result.full_text),
        created_at: chrono::Utc::now().to_rfc3339(),
        provenance_record_id: String::new(),
    };
    store::insert_artifact(db, &ocr_artifact).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    // 5. Chunk text with page anchors
    let page_chunks =
        chunk_with_page_anchors(&extract_result.full_text, &extract_result.page_offsets);

    // 6. Build chunk artifacts keyed to the OCR-result artifact
    let chunk_artifacts = build_chunk_artifacts(document_id, &ocr_artifact_id, &page_chunks);
    for chunk in &chunk_artifacts {
        store::insert_chunk(db, chunk).map_err(|e| DocsError::Storage {
            message: e.to_string(),
        })?;
    }

    // 7. Build and store provenance edges (chunk→page, artifact→document)
    let edges = build_doc_lineage_edges(document_id, &ocr_artifact_id, &chunk_artifacts, &page_ids);
    for edge in &edges {
        store::insert_provenance_edge(db, edge).map_err(|e| DocsError::Storage {
            message: e.to_string(),
        })?;
    }

    // 8. Eager indexing: embed and store vectors if a provider is configured.
    if let Some(_provider) = embed::get_provider() {
        for chunk in &chunk_artifacts {
            if let Err(e) = retrieval::index_chunk(db, chunk) {
                tracing::warn!(
                    chunk_id = %chunk.chunk_id,
                    error = %e,
                    "failed to index chunk during ingest"
                );
            }
        }
    } else {
        tracing::info!("no embedding provider configured; skipping eager indexing during ingest");
    }

    Ok(())
}

/// Ingest a directory: walk all files, ingest supported types, skip duplicates.
pub async fn ingest_directory(
    db: &DbInstance,
    dir: &Path,
) -> Result<IngestResult> {
    ensure_doc_schema(db).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    let mut result = IngestResult::default();

    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| {
            // Never skip the root directory; skip hidden subdirectories
            e.depth() == 0
                || !e.file_name()
                    .to_str()
                    .map(|s| s.starts_with('.'))
                    .unwrap_or(false)
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                result.errors.push(e.to_string());
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let media_type = detect_media_type(path);

        if !is_supported_media_type(media_type) {
            continue; // skip unsupported, no error
        }

        match ingest_file(db, path).await {
            Ok(r) if r.was_new && r.ocr_skipped => {
                result.sources_registered += 1;
                result.images_skipped += 1;
            }
            Ok(r) if r.was_new => {
                result.sources_registered += 1;
            }
            Ok(_) => {
                result.sources_skipped_duplicate += 1;
            }
            Err(e) => {
                result.sources_failed += 1;
                result.errors.push(format!("{}: {}", path.display(), e));
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-ingest-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        ensure_doc_schema(&db).unwrap();
        db
    }

    #[test]
    fn test_detect_media_type() {
        assert_eq!(detect_media_type(Path::new("doc.pdf")), "application/pdf");
        assert_eq!(detect_media_type(Path::new("readme.md")), "text/markdown");
        assert_eq!(
            detect_media_type(Path::new("notes.txt")),
            "text/plain"
        );
        assert_eq!(detect_media_type(Path::new("img.png")), "image/png");
        assert_eq!(
            detect_media_type(Path::new("unknown.xyz")),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_is_supported() {
        assert!(is_supported_media_type("text/plain"));
        assert!(is_supported_media_type("text/markdown"));
        assert!(is_supported_media_type("application/pdf"));
        assert!(!is_supported_media_type("application/octet-stream"));
        assert!(!is_supported_media_type(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        ));
    }

    #[tokio::test]
    async fn test_ingest_text_file() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "Hello world").unwrap();

        let r = ingest_file(&db, &file_path).await.unwrap();
        assert!(r.was_new);
        assert!(r.document_id.starts_with("doc-"));
        let doc_id = r.document_id;

        // Verify source in Cozo
        let doc = store::get_doc_source(&db, &doc_id).unwrap().unwrap();
        assert_eq!(doc.media_type, "text/plain");
        assert_eq!(doc.status, DocumentStatus::Ingested);
        assert!(!doc.content_hash.is_empty());

        // Verify duplicate detection — returns existing doc_id, not empty
        let dup = ingest_file(&db, &file_path).await.unwrap();
        assert!(!dup.was_new);
        assert_eq!(dup.document_id, doc_id);
    }

    #[tokio::test]
    async fn test_ingest_empty_file() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("empty.txt");
        fs::write(&file_path, "").unwrap();

        let r = ingest_file(&db, &file_path).await.unwrap();
        assert!(r.was_new);

        let sources = store::list_doc_sources(&db).unwrap();
        assert_eq!(sources.len(), 1);
    }

    #[tokio::test]
    async fn test_ingest_unsupported_format() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("data.bin");
        fs::write(&file_path, b"binary content").unwrap();

        let result = ingest_file(&db, &file_path).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            DocsError::UnsupportedMediaType { .. } => {}
            e => panic!("expected UnsupportedMediaType, got {}", e),
        }
    }

    #[tokio::test]
    async fn test_ingest_directory() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "content a").unwrap();
        fs::write(dir.path().join("b.md"), "# content b").unwrap();
        fs::write(dir.path().join("skip.bin"), "binary").unwrap();

        let result = ingest_directory(&db, dir.path()).await.unwrap();
        assert_eq!(result.sources_registered, 2); // a.txt + b.md
        assert_eq!(result.sources_skipped_duplicate, 0);
        assert_eq!(result.sources_failed, 0);

        let sources = store::list_doc_sources(&db).unwrap();
        assert_eq!(sources.len(), 2);
    }

    #[tokio::test]
    async fn test_ingest_directory_with_duplicate() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("first.txt"), "same content").unwrap();
        fs::write(dir.path().join("second.txt"), "same content").unwrap();

        let result = ingest_directory(&db, dir.path()).await.unwrap();
        assert_eq!(result.sources_registered, 1);
        assert_eq!(result.sources_skipped_duplicate, 1);

        let sources = store::list_doc_sources(&db).unwrap();
        assert_eq!(sources.len(), 1);
    }

    #[tokio::test]
    async fn test_ingest_produces_pages_chunks_and_edges() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("doc.txt");
        fs::write(
            &file_path,
            "First paragraph with enough content to matter.\n\n\
             Second paragraph also has some good content here.\n\n\
             Third paragraph is here too.",
        )
        .unwrap();

        let r = ingest_file(&db, &file_path).await.unwrap();
        assert!(r.was_new);
        let doc_id = r.document_id;

        // Status must be Ingested
        let doc = store::get_doc_source(&db, &doc_id).unwrap().unwrap();
        assert_eq!(doc.status, DocumentStatus::Ingested);

        // Must have pages
        let pages = store::list_pages_for_doc(&db, &doc_id).unwrap();
        assert!(!pages.is_empty(), "expected at least one page");
        assert_eq!(pages[0].page_number, 1);

        // Must have OCR run
        let ocr_runs = store::list_ocr_runs_for_doc(&db, &doc_id).unwrap();
        assert!(!ocr_runs.is_empty(), "expected at least one OCR run");
        assert_eq!(ocr_runs[0].status, OcrStatus::Completed);

        // Chunks may be 1 for short text (<200 chars) but must exist
        let chunks = store::list_chunks_for_doc(&db, &doc_id).unwrap();
        assert!(!chunks.is_empty(), "expected at least one chunk");

        // Must have provenance edges connected to this document
        let edges_from_chunks = store::list_provenance_from(&db, &chunks[0].chunk_id).unwrap();
        assert!(
            !edges_from_chunks.is_empty(),
            "expected chunk→page provenance edges"
        );
        // Verify an edge connects chunk to page
        let has_chunk_to_page = edges_from_chunks
            .iter()
            .any(|e| e.to_artifact_id == pages[0].page_id);
        assert!(has_chunk_to_page, "expected chunk→page edge");

        // Verify an edge connects ocr-result artifact to document
        let artifact_id = format!("ocr-result-{}", ocr_runs[0].ocr_run_id);
        let edges_from_artifact = store::list_provenance_from(&db, &artifact_id).unwrap();
        let has_artifact_to_doc = edges_from_artifact
            .iter()
            .any(|e| e.to_artifact_id == doc_id);
        assert!(has_artifact_to_doc, "expected ocr-artifact→document edge");

        // Verify inspect picks up all edges
        let inspect_output = crate::inspect::inspect_document(&db, &doc_id).unwrap();
        assert!(
            inspect_output.provenance_edges.len() >= 2,
            "inspect must surface ≥2 edges, got {}",
            inspect_output.provenance_edges.len()
        );
    }

    #[tokio::test]
    async fn test_ingest_image_skips_ocr() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        // Create a minimal valid PNG file (89 50 4E 47 magic bytes)
        let file_path = dir.path().join("test.png");
        let png_bytes = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
            0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
            0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
            0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
            0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41,
            0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
            0x00, 0x00, 0x02, 0x00, 0x01, 0xE2, 0x21, 0xBC,
            0x33, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
            0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        fs::write(&file_path, png_bytes).unwrap();

        let r = ingest_file(&db, &file_path).await.unwrap();
        assert!(r.was_new);
        assert!(r.ocr_skipped);

        // Document status should be Ingested
        let doc = store::get_doc_source(&db, &r.document_id).unwrap().unwrap();
        assert_eq!(doc.status, DocumentStatus::Ingested);

        // No pages/chunks for image ingest
        let pages = store::list_pages_for_doc(&db, &r.document_id).unwrap();
        assert!(pages.is_empty(), "images should have no pages");
        let chunks = store::list_chunks_for_doc(&db, &r.document_id).unwrap();
        assert!(chunks.is_empty(), "images should have no chunks");

        // Directory ingest: images_skipped counter (unique content to avoid duplicate)
        let dir2 = tempfile::tempdir().unwrap();
        fs::write(dir2.path().join("img2.png"), b"PNG_PLACEHOLDER_2").unwrap();
        let dir_result = ingest_directory(&db, dir2.path()).await.unwrap();
        assert_eq!(dir_result.images_skipped, 1);
    }

    #[tokio::test]
    async fn test_ingest_pipeline_failure_sets_failed_status() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        // Garbage .pdf that pdftotext will reject
        let file_path = dir.path().join("bad.pdf");
        fs::write(&file_path, b"this is not a valid PDF").unwrap();

        let r = ingest_file(&db, &file_path).await.unwrap();
        assert!(r.was_new);

        let doc = store::get_doc_source(&db, &r.document_id).unwrap().unwrap();
        assert_eq!(
            doc.status,
            DocumentStatus::Failed,
            "pipeline failure must set Failed status"
        );
    }

    // ── BLOCKER #1: Eager indexing tests ─────────────────────────────

    use crate::embed::{self, LocalEmbeddingProvider};

    struct IndexingMockProvider {
        dim: usize,
        // If set, embed_chunks returns this error.
        fail_with: Option<String>,
    }

    impl LocalEmbeddingProvider for IndexingMockProvider {
        fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
            if let Some(ref msg) = self.fail_with {
                return Err(DocsError::Embedding { message: msg.clone() });
            }
            Ok(chunks
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let mut v = vec![0.0_f32; self.dim];
                    for (j, b) in c.bytes().enumerate() {
                        v[j % self.dim] = (b as f32) / 255.0;
                    }
                    v[0] = (i as f32 + 1.0) * 0.5;
                    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
                    v.iter_mut().for_each(|x| *x /= norm);
                    v
                })
                .collect())
        }

        fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
            let mut results = self.embed_chunks(&[query.to_string()])?;
            Ok(results.remove(0))
        }

        fn dimension(&self) -> usize {
            self.dim
        }

        fn backend_name(&self) -> &'static str {
            "mock-indexing"
        }
    }

    #[tokio::test]
    async fn test_ingest_indexes_chunks_when_provider_set() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        // Multi-chunk fixture: enough content for several chunks
        let content = (0..50)
            .map(|i| format!("Paragraph {} with enough text content to fill multiple chunks in the pipeline.\n", i))
            .collect::<Vec<_>>()
            .join("\n");
        let file_path = dir.path().join("multi.txt");
        fs::write(&file_path, &content).unwrap();

        embed::set_provider(Box::new(IndexingMockProvider { dim: 4, fail_with: None }));

        let r = ingest_file(&db, &file_path).await.unwrap();
        assert!(r.was_new);

        let chunks = store::list_chunks_for_doc(&db, &r.document_id).unwrap();
        assert!(!chunks.is_empty(), "expected at least one chunk");
        for chunk in &chunks {
            assert_eq!(
                chunk.embedding_status, "indexed",
                "chunk {} should be indexed, got {}",
                chunk.chunk_id, chunk.embedding_status
            );
        }
        let count = store::count_embeddings(&db).unwrap();
        assert_eq!(count, chunks.len(), "all chunks should have embeddings");
    }

    #[tokio::test]
    async fn test_ingest_succeeds_without_provider() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("noprov.txt");
        fs::write(&file_path, "Some content for a document without embedding provider.\n").unwrap();

        // Ensure no provider is set
        embed::clear_provider();

        let r = ingest_file(&db, &file_path).await.unwrap();
        assert!(r.was_new);

        let chunks = store::list_chunks_for_doc(&db, &r.document_id).unwrap();
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            assert_eq!(chunk.embedding_status, "pending");
        }
        assert_eq!(store::count_embeddings(&db).unwrap(), 0);
    }

    #[tokio::test]
    async fn test_second_ingest_indexes_new_chunks() {
        let db = test_db();
        embed::set_provider(Box::new(IndexingMockProvider { dim: 4, fail_with: None }));

        // Doc A
        let dir = tempfile::tempdir().unwrap();
        let path_a = dir.path().join("doc_a.txt");
        fs::write(&path_a, "Document A content with some text for chunking.\n").unwrap();
        let r_a = ingest_file(&db, &path_a).await.unwrap();
        assert!(r_a.was_new);

        let chunks_a = store::list_chunks_for_doc(&db, &r_a.document_id).unwrap();
        let count_after_a = store::count_embeddings(&db).unwrap();
        assert_eq!(count_after_a, chunks_a.len());

        // Doc B
        let path_b = dir.path().join("doc_b.txt");
        fs::write(&path_b, "Document B with different content for another ingest test.\n").unwrap();
        let r_b = ingest_file(&db, &path_b).await.unwrap();
        assert!(r_b.was_new);

        let chunks_b = store::list_chunks_for_doc(&db, &r_b.document_id).unwrap();
        let count_after_b = store::count_embeddings(&db).unwrap();
        assert_eq!(count_after_b, chunks_a.len() + chunks_b.len(), "both doc A and B chunks should be embedded");

        for chunk in &chunks_b {
            assert_eq!(chunk.embedding_status, "indexed");
        }
    }

    #[tokio::test]
    async fn test_index_failure_marks_chunk_failed() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("fail.txt");
        fs::write(&file_path, "Content that will fail to embed.\n").unwrap();

        embed::set_provider(Box::new(IndexingMockProvider {
            dim: 4,
            fail_with: Some("simulated embedding failure".into()),
        }));

        let r = ingest_file(&db, &file_path).await.unwrap();
        assert!(r.was_new);

        let chunks = store::list_chunks_for_doc(&db, &r.document_id).unwrap();
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            assert_eq!(
                chunk.embedding_status, "failed",
                "chunk {} should be marked failed after embed error, got {}",
                chunk.chunk_id, chunk.embedding_status
            );
        }
        assert_eq!(store::count_embeddings(&db).unwrap(), 0);
    }
}
