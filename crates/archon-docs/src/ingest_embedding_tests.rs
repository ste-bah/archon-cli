use super::test_support::*;
use super::*;

use crate::embed;

#[tokio::test]
async fn test_ingest_indexes_chunks_when_provider_set() {
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    // Multi-chunk fixture: enough content for several chunks
    let content = (0..50)
        .map(|i| {
            format!(
                "Paragraph {} with enough text content to fill multiple chunks in the pipeline.\n",
                i
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let file_path = dir.path().join("multi.txt");
    fs::write(&file_path, &content).unwrap();

    embed::set_provider(Box::new(IndexingMockProvider {
        dim: 4,
        fail_with: None,
    }));

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
async fn test_image_embedding_stored_when_provider_is_multimodal() {
    reset_multimodal_test_providers();
    crate::ocr::provider::set_provider(Box::new(MockOcrProvider {
        text: "OCR text for multimodal embedding",
    }));
    embed::set_provider(Box::new(MultimodalMockProvider { dim: 4 }));

    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("vision.png");
    fs::write(&file_path, b"PNG_IMAGE_EMBEDDING").unwrap();

    let r = ingest_file(&db, &file_path).await.unwrap();
    assert_eq!(r.image_embeddings_stored, 1);
    assert_eq!(store::count_page_image_embeddings(&db).unwrap(), 1);

    let chunks = store::list_chunks_for_doc(&db, &r.document_id).unwrap();
    assert!(!chunks.is_empty());
    assert_eq!(store::count_embeddings(&db).unwrap(), chunks.len());
    reset_multimodal_test_providers();
}

#[tokio::test]
async fn test_ingest_succeeds_without_provider() {
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("noprov.txt");
    fs::write(
        &file_path,
        "Some content for a document without embedding provider.\n",
    )
    .unwrap();

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
    embed::set_provider(Box::new(IndexingMockProvider {
        dim: 4,
        fail_with: None,
    }));

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
    fs::write(
        &path_b,
        "Document B with different content for another ingest test.\n",
    )
    .unwrap();
    let r_b = ingest_file(&db, &path_b).await.unwrap();
    assert!(r_b.was_new);

    let chunks_b = store::list_chunks_for_doc(&db, &r_b.document_id).unwrap();
    let count_after_b = store::count_embeddings(&db).unwrap();
    assert_eq!(
        count_after_b,
        chunks_a.len() + chunks_b.len(),
        "both doc A and B chunks should be embedded"
    );

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
