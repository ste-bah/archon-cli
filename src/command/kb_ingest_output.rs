use anyhow::Result;
use cozo::DbInstance;

pub(crate) fn print_file_result(
    db: &DbInstance,
    result: &archon_docs::ingest::IngestFileResult,
    vlm_report: &archon_docs::vlm::factory::VlmProviderInitReport,
) -> Result<()> {
    let chunks = archon_docs::store::list_chunks_for_doc(db, &result.document_id)?;
    if result.was_new {
        println!("Ingested: {}", result.document_id);
    } else {
        println!("Skipped duplicate: true");
        println!("Ingested: {}", result.document_id);
    }
    println!("Chunks: {}", chunks.len());
    if result.vlm_descriptions > 0 {
        println!(
            "VLM descriptions: {} via {}/{}",
            result.vlm_descriptions, vlm_report.provider, vlm_report.model
        );
    }
    print_file_pdf_metrics(result);
    if result.pipeline_failed {
        println!("Warning: processing failed; document status is Failed");
    }
    print_vlm_init_status(vlm_report);
    for warning in &result.warnings {
        println!("Warning: {warning}");
    }
    Ok(())
}

pub(crate) fn print_directory_result(
    result: &archon_docs::ingest::IngestResult,
    vlm_report: &archon_docs::vlm::factory::VlmProviderInitReport,
) {
    println!("Ingested: {} sources", result.sources_registered);
    println!("Skipped duplicates: {}", result.sources_skipped_duplicate);
    println!("Failed: {}", result.sources_failed);
    if result.vlm_descriptions > 0 {
        println!(
            "VLM described: {} image file(s) via {}/{}",
            result.vlm_descriptions, vlm_report.provider, vlm_report.model
        );
    }
    print_directory_pdf_metrics(result);
    print_vlm_init_status(vlm_report);
    for warning in &result.warnings {
        println!("Warning: {warning}");
    }
}

pub(crate) fn print_vlm_init_status(report: &archon_docs::vlm::factory::VlmProviderInitReport) {
    match report.status {
        archon_docs::vlm::factory::VlmProviderInitStatus::Registered => {
            println!("VLM provider: {}/{}", report.provider, report.model);
        }
        archon_docs::vlm::factory::VlmProviderInitStatus::Skipped => {
            println!("Warning: {}", report.message);
        }
        archon_docs::vlm::factory::VlmProviderInitStatus::Disabled => {}
    }
}

fn print_file_pdf_metrics(result: &archon_docs::ingest::IngestFileResult) {
    if result.pdf_embedded_images_extracted == 0 && result.pdf_pages_rendered == 0 {
        return;
    }
    println!(
        "PDF images: {} embedded extracted, {} skipped by filter, {} rendered page(s)",
        result.pdf_embedded_images_extracted,
        result.pdf_embedded_images_skipped_filter,
        result.pdf_pages_rendered
    );
    println!(
        "PDF image OCR: {} run(s), {} failure(s); VLM failures: {}",
        result.pdf_image_ocr_runs, result.pdf_image_ocr_failures, result.pdf_image_vlm_failures
    );
}

fn print_directory_pdf_metrics(result: &archon_docs::ingest::IngestResult) {
    if result.pdf_embedded_images_extracted == 0 && result.pdf_pages_rendered == 0 {
        return;
    }
    println!(
        "PDF images: {} embedded extracted, {} skipped by filter, {} rendered page(s)",
        result.pdf_embedded_images_extracted,
        result.pdf_embedded_images_skipped_filter,
        result.pdf_pages_rendered
    );
    println!(
        "PDF image OCR: {} run(s), {} failure(s); VLM failures: {}",
        result.pdf_image_ocr_runs, result.pdf_image_ocr_failures, result.pdf_image_vlm_failures
    );
}
