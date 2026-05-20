use super::test_support::*;
use super::*;

#[test]
fn test_detect_media_type() {
    assert_eq!(detect_media_type(Path::new("doc.pdf")), "application/pdf");
    assert_eq!(detect_media_type(Path::new("readme.md")), "text/markdown");
    assert_eq!(detect_media_type(Path::new("notes.txt")), "text/plain");
    assert_eq!(detect_media_type(Path::new("page.html")), "text/html");
    assert_eq!(
        detect_media_type(Path::new("data.json")),
        "application/json"
    );
    assert_eq!(detect_media_type(Path::new("feed.xml")), "application/xml");
    assert_eq!(detect_media_type(Path::new("cfg.yaml")), "application/yaml");
    assert_eq!(detect_media_type(Path::new("app.toml")), "application/toml");
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
    assert!(is_supported_media_type("text/html"));
    assert!(is_supported_media_type("application/json"));
    assert!(is_supported_media_type("application/xml"));
    assert!(is_supported_media_type("application/yaml"));
    assert!(is_supported_media_type("application/toml"));
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
