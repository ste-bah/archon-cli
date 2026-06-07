use super::test_support::*;
use super::*;

use crate::models::ProvenanceEdgeType;

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn test_pdf_ingest_persists_embedded_image_ocr_and_vlm_description() {
    reset_multimodal_test_providers();
    crate::ocr::provider::set_provider(Box::new(MockOcrProvider {
        text: "axis label OCR from PDF chart",
    }));
    crate::vlm::set_provider(Box::new(MockVlmProvider {
        description: "ascending triangle with rising volume",
    }));
    let policy = vlm_enabled_policy();
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("chart-book.pdf");
    fs::write(&pdf, b"%PDF mixed chart book").unwrap();
    let pdftotext = dir.path().join("pdftotext");
    let pdfimages = dir.path().join("pdfimages");
    let pdftoppm = dir.path().join("pdftoppm");
    let chart = dir.path().join("chart.bin");
    fs::write(&chart, png_bytes(800, 600, 8192)).unwrap();
    write_executable(
        &pdftotext,
        "#!/usr/bin/env bash\necho 'body text discusses waves'\n",
    );
    write_executable(
        &pdfimages,
        &format!(
            "#!/usr/bin/env bash\n\
             if [ \"$1\" = \"-list\" ]; then echo '  1 0 image 800 600 rgb 3 8 image no 12 0 72 72 8K 1%'; exit 0; fi\n\
             cp '{}' \"${{@: -1}}-000.png\"\n",
            chart.display()
        ),
    );
    write_executable(&pdftoppm, "#!/usr/bin/env bash\nexit 99\n");
    set_pdf_command_env(&pdftotext, &pdfimages, &pdftoppm);
    let _guard = PdfCommandEnvGuard;

    let result = ingest_file_with_policy(&db, &pdf, &policy).await.unwrap();
    assert_eq!(result.pdf_embedded_images_extracted, 1);
    assert_eq!(result.pdf_image_ocr_runs, 1);
    assert_eq!(result.vlm_descriptions, 1);

    let chunks = store::list_chunks_for_doc(&db, &result.document_id).unwrap();
    let joined = chunks
        .iter()
        .map(|chunk| chunk.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("body text discusses waves"));
    assert!(joined.contains("axis label OCR from PDF chart"));
    assert!(joined.contains("ascending triangle with rising volume"));

    let descriptions = store::list_image_descriptions_for_doc(&db, &result.document_id).unwrap();
    assert_eq!(descriptions.len(), 1);
    assert_eq!(descriptions[0].page_number, 1);
    let metrics = store::get_pdf_metrics(&db, &result.document_id)
        .unwrap()
        .unwrap();
    assert_eq!(metrics.embedded_images_extracted, 1);
    assert_eq!(metrics.image_ocr_runs, 1);
    assert_eq!(metrics.image_vlm_descriptions, 1);
    reset_multimodal_test_providers();
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn test_pdf_ingest_scanned_rendered_pages_get_vlm_descriptions() {
    reset_multimodal_test_providers();
    crate::ocr::provider::set_provider(Box::new(MockOcrProvider {
        text: "rendered page OCR",
    }));
    crate::vlm::set_provider(Box::new(MockVlmProvider {
        description: "rendered page visual description",
    }));
    let policy = vlm_enabled_policy();
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("scan.pdf");
    fs::write(&pdf, b"%PDF scan").unwrap();
    let pdftotext = dir.path().join("pdftotext");
    let pdfimages = dir.path().join("pdfimages");
    let pdftoppm = dir.path().join("pdftoppm");
    let page = dir.path().join("page.bin");
    fs::write(&page, png_bytes(640, 480, 4096)).unwrap();
    write_executable(&pdftotext, "#!/usr/bin/env bash\nexit 0\n");
    write_executable(&pdfimages, "#!/usr/bin/env bash\nexit 0\n");
    write_executable(
        &pdftoppm,
        &format!(
            "#!/usr/bin/env bash\n\
             cp '{}' \"${{@: -1}}-1.png\"\n\
             cp '{}' \"${{@: -1}}-2.png\"\n\
             cp '{}' \"${{@: -1}}-3.png\"\n",
            page.display(),
            page.display(),
            page.display()
        ),
    );
    set_pdf_command_env(&pdftotext, &pdfimages, &pdftoppm);
    let _guard = PdfCommandEnvGuard;

    let result = ingest_file_with_policy(&db, &pdf, &policy).await.unwrap();
    assert_eq!(result.pdf_pages_rendered, 3);
    assert_eq!(result.pdf_image_ocr_runs, 3);
    assert_eq!(result.vlm_descriptions, 3);
    let descriptions = store::list_image_descriptions_for_doc(&db, &result.document_id).unwrap();
    assert_eq!(descriptions.len(), 3);
    reset_multimodal_test_providers();
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn test_pdf_single_image_vlm_failure_does_not_fail_document() {
    reset_multimodal_test_providers();
    crate::ocr::provider::set_provider(Box::new(MockOcrProvider {
        text: "image OCR survives partial VLM failure",
    }));
    crate::vlm::set_provider(Box::new(FailsOnceVlmProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
    }));
    let policy = vlm_enabled_policy();
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("two-images.pdf");
    fs::write(&pdf, b"%PDF two charts").unwrap();
    let pdftotext = dir.path().join("pdftotext");
    let pdfimages = dir.path().join("pdfimages");
    let pdftoppm = dir.path().join("pdftoppm");
    let chart_a = dir.path().join("chart-a.bin");
    let chart_b = dir.path().join("chart-b.bin");
    fs::write(&chart_a, png_bytes(800, 600, 8192)).unwrap();
    fs::write(&chart_b, png_bytes(801, 600, 8192)).unwrap();
    write_executable(&pdftotext, "#!/usr/bin/env bash\necho 'body text'\n");
    write_executable(
        &pdfimages,
        &format!(
            "#!/usr/bin/env bash\n\
             if [ \"$1\" = \"-list\" ]; then\n\
               echo '  1 0 image 800 600 rgb 3 8 image no 12 0 72 72 8K 1%'\n\
               echo '  1 1 image 801 600 rgb 3 8 image no 13 0 72 72 8K 1%'\n\
               exit 0\n\
             fi\n\
             cp '{}' \"${{@: -1}}-000.png\"\n\
             cp '{}' \"${{@: -1}}-001.png\"\n",
            chart_a.display(),
            chart_b.display()
        ),
    );
    write_executable(&pdftoppm, "#!/usr/bin/env bash\nexit 99\n");
    set_pdf_command_env(&pdftotext, &pdfimages, &pdftoppm);
    let _guard = PdfCommandEnvGuard;

    let result = ingest_file_with_policy(&db, &pdf, &policy).await.unwrap();
    assert!(!result.pipeline_failed);
    assert_eq!(result.pdf_image_vlm_failures, 1);
    assert_eq!(result.vlm_descriptions, 1);
    let doc = store::get_doc_source(&db, &result.document_id)
        .unwrap()
        .unwrap();
    assert_eq!(doc.status, DocumentStatus::Ingested);
    reset_multimodal_test_providers();
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn test_pdf_vlm_provider_empty_response_retries_without_failing_document() {
    reset_multimodal_test_providers();
    crate::ocr::provider::set_provider(Box::new(MockOcrProvider { text: "chart OCR" }));
    crate::vlm::set_provider(Box::new(ProviderErrorThenOkVlmProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
    }));
    let policy = vlm_enabled_policy();
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("provider-empty-retry.pdf");
    fs::write(&pdf, b"%PDF provider retry chart").unwrap();
    let pdftotext = dir.path().join("pdftotext");
    let pdfimages = dir.path().join("pdfimages");
    let pdftoppm = dir.path().join("pdftoppm");
    let chart = dir.path().join("chart.bin");
    fs::write(&chart, png_bytes(800, 600, 8192)).unwrap();
    write_executable(&pdftotext, "#!/usr/bin/env bash\necho 'body text'\n");
    write_executable(
        &pdfimages,
        &format!(
            "#!/usr/bin/env bash\n\
             if [ \"$1\" = \"-list\" ]; then\n\
               echo '  1 0 image 800 600 rgb 3 8 image no 12 0 72 72 8K 1%'\n\
               exit 0\n\
             fi\n\
             cp '{}' \"${{@: -1}}-000.png\"\n",
            chart.display()
        ),
    );
    write_executable(&pdftoppm, "#!/usr/bin/env bash\nexit 99\n");
    set_pdf_command_env(&pdftotext, &pdfimages, &pdftoppm);
    let _guard = PdfCommandEnvGuard;

    let result = ingest_file_with_policy(&db, &pdf, &policy).await.unwrap();
    assert!(!result.pipeline_failed);
    assert_eq!(result.pdf_image_vlm_failures, 0);
    assert_eq!(result.vlm_descriptions, 1);
    let doc = store::get_doc_source(&db, &result.document_id)
        .unwrap()
        .unwrap();
    assert_eq!(doc.status, DocumentStatus::Ingested);
    reset_multimodal_test_providers();
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn test_pdf_shared_xobject_image_has_edges_to_each_source_page() {
    reset_multimodal_test_providers();
    crate::ocr::provider::set_provider(Box::new(MockOcrProvider {
        text: "shared chart OCR",
    }));
    crate::vlm::set_provider(Box::new(MockVlmProvider {
        description: "shared image description",
    }));
    let policy = vlm_enabled_policy();
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("shared.pdf");
    fs::write(&pdf, b"%PDF shared").unwrap();
    let pdftotext = dir.path().join("pdftotext");
    let pdfimages = dir.path().join("pdfimages");
    let pdftoppm = dir.path().join("pdftoppm");
    let chart = dir.path().join("shared.bin");
    fs::write(&chart, png_bytes(800, 600, 8192)).unwrap();
    write_executable(
        &pdftotext,
        "#!/usr/bin/env bash\nprintf 'page one\\fpage two'\n",
    );
    write_executable(
        &pdfimages,
        &format!(
            "#!/usr/bin/env bash\n\
             if [ \"$1\" = \"-list\" ]; then\n\
               echo '  1 0 image 800 600 rgb 3 8 image no 12 0 72 72 8K 1%'\n\
               echo '  2 1 image 800 600 rgb 3 8 image no 12 0 72 72 8K 1%'\n\
               exit 0\n\
             fi\n\
             cp '{}' \"${{@: -1}}-000.png\"\n",
            chart.display()
        ),
    );
    write_executable(&pdftoppm, "#!/usr/bin/env bash\nexit 99\n");
    set_pdf_command_env(&pdftotext, &pdfimages, &pdftoppm);
    let _guard = PdfCommandEnvGuard;

    let result = ingest_file_with_policy(&db, &pdf, &policy).await.unwrap();
    let descriptions = store::list_image_descriptions_for_doc(&db, &result.document_id).unwrap();
    assert_eq!(descriptions.len(), 1);
    let chunks = store::list_chunks_for_doc(&db, &result.document_id).unwrap();
    let description_chunk = chunks
        .iter()
        .find(|chunk| chunk.artifact_id == descriptions[0].artifact_id)
        .expect("description chunk persisted");
    let edges = store::list_provenance_from(&db, &description_chunk.chunk_id).unwrap();
    assert_eq!(
        edges
            .iter()
            .filter(|edge| matches!(edge.edge_type, ProvenanceEdgeType::Describes))
            .count(),
        2
    );
    reset_multimodal_test_providers();
}

#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn test_ingest_pipeline_failure_sets_failed_status() {
    let db = test_db();
    let dir = tempfile::tempdir().unwrap();
    // Garbage .pdf that pdftotext will reject
    let file_path = dir.path().join("bad.pdf");
    fs::write(&file_path, b"this is not a valid PDF").unwrap();

    let r = ingest_file(&db, &file_path).await.unwrap();
    assert!(r.was_new);
    assert!(r.pipeline_failed);
    assert!(
        r.warnings.iter().any(|w| w.contains("OCR pipeline failed")),
        "pipeline failure should be surfaced in result warnings"
    );

    let doc = store::get_doc_source(&db, &r.document_id).unwrap().unwrap();
    assert_eq!(
        doc.status,
        DocumentStatus::Failed,
        "pipeline failure must set Failed status"
    );
}

// ── BLOCKER #1: Eager indexing tests ─────────────────────────────
