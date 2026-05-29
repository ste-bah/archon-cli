use std::collections::BTreeMap;
use std::path::Path;

use cozo::DbInstance;

use crate::errors::DocsError;
use crate::hash::sha256_str;
use crate::ingest::PipelineOutcome;
use crate::ingest_artifacts::{index_chunks_if_provider_available, persist_text_artifact_chunks};
use crate::models::{OcrStatus, PageArtifact, PdfIngestMetrics};
use crate::pdf;
use crate::pdf_image_enrichment::enrich_pdf_images;
use crate::provenance::build_doc_lineage_edges;
use crate::store;

pub(crate) async fn run_pdf_ingest_pipeline(
    db: &DbInstance,
    document_id: &str,
    file_path: &str,
    ocr_run_id: &str,
    policy: &archon_policy::EffectivePolicy,
) -> Result<PipelineOutcome, DocsError> {
    let mut outcome = PipelineOutcome::default();
    let extract_result = pdf::extract_pdf_unified(Path::new(file_path), &policy.docs.pdf).await?;
    outcome.warnings.extend(extract_result.warnings.clone());
    outcome.pdf_embedded_images_extracted = extract_result.embedded_images.len();
    outcome.pdf_embedded_images_skipped_filter = extract_result.embedded_images_skipped_filter;
    outcome.pdf_pages_rendered = extract_result.rendered_pages.len();

    store::update_ocr_run_completion(
        db,
        ocr_run_id,
        &OcrStatus::Completed,
        &chrono::Utc::now().to_rfc3339(),
        extract_result.processing_duration_ms,
    )
    .map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    let mut page_ids_by_number = BTreeMap::<u32, String>::new();
    let mut pages_by_number = BTreeMap::<u32, PageArtifact>::new();
    for po in &extract_result.page_offsets {
        let page_id = format!("page-{}-{}", document_id, po.page);
        let page_text = extract_result
            .full_text
            .get(po.char_start..po.char_end)
            .unwrap_or("");
        let page = PageArtifact {
            page_id: page_id.clone(),
            document_id: document_id.to_string(),
            page_number: po.page,
            text_hash: if page_text.trim().is_empty() {
                None
            } else {
                Some(sha256_str(page_text))
            },
            image_hash: None,
            width: None,
            height: None,
            provenance_record_id: String::new(),
        };
        store::insert_page(db, &page).map_err(|e| DocsError::Storage {
            message: e.to_string(),
        })?;
        page_ids_by_number.insert(po.page, page_id);
        pages_by_number.insert(po.page, page);
    }

    let page_ids = page_ids_by_number.values().cloned().collect::<Vec<_>>();
    if !extract_result.full_text.trim().is_empty() {
        let ocr_artifact_id = format!("ocr-result-{}", ocr_run_id);
        let chunks = persist_text_artifact_chunks(
            db,
            document_id,
            &ocr_artifact_id,
            "ocr_text",
            &extract_result.full_text,
            &extract_result.page_offsets,
            None,
        )?;
        let edges = build_doc_lineage_edges(document_id, &ocr_artifact_id, &chunks, &page_ids);
        for edge in &edges {
            store::insert_provenance_edge(db, edge).map_err(|e| DocsError::Storage {
                message: e.to_string(),
            })?;
        }
        index_chunks_if_provider_available(db, &chunks);
    }

    let pdf_images = extract_result
        .embedded_images
        .iter()
        .chain(extract_result.rendered_pages.iter())
        .cloned()
        .collect::<Vec<_>>();
    if policy.docs.pdf.vlm_per_page_image
        && policy.docs.vlm.provider != "ollama"
        && policy.docs.vlm.provider != "disabled"
    {
        tracing::info!(
            images = pdf_images.len(),
            provider = %policy.docs.vlm.provider,
            "PDF ingest will trigger VLM calls for extracted page images"
        );
    }
    enrich_pdf_images(
        db,
        document_id,
        &pdf_images,
        policy,
        &page_ids_by_number,
        &mut pages_by_number,
        &mut outcome,
    )
    .await?;
    outcome.pdf_embedded_images_extracted = outcome
        .pdf_embedded_images_extracted
        .max(extract_result.embedded_images.len());

    outcome.pdf_embedded_images_skipped_filter = extract_result.embedded_images_skipped_filter;
    let metrics = PdfIngestMetrics {
        document_id: document_id.to_string(),
        embedded_images_extracted: outcome.pdf_embedded_images_extracted as u32,
        embedded_images_skipped_filter: outcome.pdf_embedded_images_skipped_filter as u32,
        image_ocr_runs: outcome.pdf_image_ocr_runs as u32,
        image_ocr_failures: outcome.pdf_image_ocr_failures as u32,
        image_vlm_descriptions: outcome.vlm_descriptions as u32,
        image_vlm_failures: outcome.pdf_image_vlm_failures as u32,
        pages_rendered: outcome.pdf_pages_rendered as u32,
    };
    store::upsert_pdf_metrics(db, &metrics).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    Ok(outcome)
}
