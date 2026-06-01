use super::test_support::*;
use super::*;

#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn test_ingest_image_runs_ocr_and_persists_rows() {
    reset_multimodal_test_providers();
    crate::ocr::provider::set_provider(Box::new(MockOcrProvider {
        text: "SYNTHETIC OCR TEXT from image",
    }));
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    // Create a minimal valid PNG file (89 50 4E 47 magic bytes)
    let file_path = dir.path().join("test.png");
    let png_bytes = [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0xE2, 0x21, 0xBC, 0x33, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    fs::write(&file_path, png_bytes).unwrap();

    let r = ingest_file(&db, &file_path).await.unwrap();
    assert!(r.was_new);
    assert!(!r.ocr_skipped);

    // Document status should be Ingested
    let doc = store::get_doc_source(&db, &r.document_id).unwrap().unwrap();
    assert_eq!(doc.status, DocumentStatus::Ingested);

    // Source of truth: OCR/page/chunk rows are physically present in Cozo.
    let ocr_runs = store::list_ocr_runs_for_doc(&db, &r.document_id).unwrap();
    assert_eq!(ocr_runs.len(), 1);
    assert_eq!(ocr_runs[0].provider, "mock-ocr");
    assert_eq!(ocr_runs[0].status, OcrStatus::Completed);

    let pages = store::list_pages_for_doc(&db, &r.document_id).unwrap();
    assert_eq!(pages.len(), 1, "image OCR should create a page row");
    assert!(
        pages[0].image_hash.is_some(),
        "image page must retain source image hash"
    );

    let chunks = store::list_chunks_for_doc(&db, &r.document_id).unwrap();
    assert_eq!(chunks.len(), 1, "image OCR should create a text chunk");
    assert!(chunks[0].content.contains("SYNTHETIC OCR TEXT"));

    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("image embedding skipped: no embedding provider configured")),
        "text-only/no-provider image embedding path must report an explicit warning"
    );

    // Directory ingest: image_ocr_completed counter (unique content to avoid duplicate)
    let dir2 = tempfile::tempdir().unwrap();
    fs::write(dir2.path().join("img2.png"), b"PNG_PLACEHOLDER_2").unwrap();
    let dir_result = ingest_directory(&db, dir2.path()).await.unwrap();
    assert_eq!(dir_result.images_skipped, 0);
    assert_eq!(dir_result.image_ocr_completed, 1);
    reset_multimodal_test_providers();
}

#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn test_vlm_disabled_by_default_does_not_describe_image() {
    reset_multimodal_test_providers();
    crate::ocr::provider::set_provider(Box::new(MockOcrProvider {
        text: "OCR only text",
    }));
    crate::vlm::set_provider(Box::new(MockVlmProvider {
        description: "a policy-gated chart description",
    }));
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("chart.png");
    fs::write(&file_path, b"PNG_DEFAULT_VLM_DISABLED").unwrap();

    let r = ingest_file(&db, &file_path).await.unwrap();
    assert_eq!(r.vlm_descriptions, 0);
    let chunks = store::list_chunks_for_doc(&db, &r.document_id).unwrap();
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].content.contains("OCR only text"));
    assert!(!chunks[0].content.contains("policy-gated chart description"));
    reset_multimodal_test_providers();
}

#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn test_vlm_enabled_adds_description_to_image_chunks() {
    reset_multimodal_test_providers();
    crate::ocr::provider::set_provider(Box::new(MockOcrProvider {
        text: "OCR text before VLM",
    }));
    crate::vlm::set_provider(Box::new(MockVlmProvider {
        description: "diagram shows a synthetic reward loop",
    }));
    let mut policy = archon_policy::EffectivePolicy::default();
    policy.docs.vlm.enabled = true;
    policy.docs.vlm.mode = "local".into();
    policy.docs.vlm.provider = "ollama".into();
    policy.workers.vlm = "allow-local".into();

    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("diagram.png");
    fs::write(&file_path, b"PNG_VLM_ENABLED").unwrap();

    let r = ingest_file_with_policy(&db, &file_path, &policy)
        .await
        .unwrap();
    assert_eq!(r.vlm_descriptions, 1);
    let chunks = store::list_chunks_for_doc(&db, &r.document_id).unwrap();
    let joined = chunks
        .iter()
        .map(|chunk| chunk.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("diagram shows a synthetic reward loop"));
    let image_descriptions = store::list_image_descriptions_for_doc(&db, &r.document_id).unwrap();
    assert_eq!(image_descriptions.len(), 1);
    assert_eq!(
        image_descriptions[0].description,
        "diagram shows a synthetic reward loop"
    );
    reset_multimodal_test_providers();
}

#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn test_vlm_provider_failure_warns_but_keeps_ocr_ingest() {
    reset_multimodal_test_providers();
    crate::ocr::provider::set_provider(Box::new(MockOcrProvider {
        text: "OCR survives VLM failure",
    }));
    crate::vlm::set_provider(Box::new(FailingVlmProvider));
    let mut policy = archon_policy::EffectivePolicy::default();
    policy.docs.vlm.enabled = true;
    policy.docs.vlm.mode = "local".into();
    policy.docs.vlm.provider = "ollama".into();
    policy.workers.vlm = "allow-local".into();

    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("vlm-fail.png");
    fs::write(&file_path, b"PNG_VLM_FAILURE").unwrap();

    let r = ingest_file_with_policy(&db, &file_path, &policy)
        .await
        .unwrap();
    assert!(!r.pipeline_failed);
    assert_eq!(r.vlm_descriptions, 0);
    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("image description failed"))
    );
    let doc = store::get_doc_source(&db, &r.document_id).unwrap().unwrap();
    assert_eq!(doc.status, DocumentStatus::Ingested);
    let chunks = store::list_chunks_for_doc(&db, &r.document_id).unwrap();
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].content.contains("OCR survives VLM failure"));
    reset_multimodal_test_providers();
}
