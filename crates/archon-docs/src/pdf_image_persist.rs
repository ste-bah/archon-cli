//! Persistence for enriched PDF images: OCR runs, VLM descriptions, and the derived
//! OCR chunks folded into the document's chunks_root.
//!
//! Split out of `pdf_image_enrichment.rs` to keep both files under the 500-line gate.

use super::*;

pub(super) fn persist_image_result(
    db: &DbInstance,
    document_id: &str,
    result: ImageResult,
    outcome: &mut PipelineOutcome,
    collected: &mut Vec<ChunkArtifact>,
) -> Result<(), DocsError> {
    persist_ocr_result(db, document_id, &result, outcome, collected)?;
    if let Some(vlm) = result.vlm {
        persist_vlm_result(db, document_id, &result.work, vlm, outcome, collected)?;
    }
    // Full coverage: CLIP-embed each embedded PDF figure so they are visually searchable
    // alongside standalone images. Key per-figure ("{page_id}-img{N}") so multiple figures
    // on one page don't collide; `retrieval_image::resolve_page` strips the suffix back to
    // the page for result resolution. (Suppress the per-figure "not multimodal" warning.)
    if let Some(page_id) = result.work.page_ids.first() {
        let fig_key = format!("{}-img{}", page_id, result.work.current);
        crate::ingest_multimodal::store_image_embedding_if_supported(
            db,
            &[fig_key],
            &result.work.image.bytes,
            true,
            outcome,
        );
    }
    Ok(())
}

fn persist_ocr_result(
    db: &DbInstance,
    document_id: &str,
    result: &ImageResult,
    outcome: &mut PipelineOutcome,
    collected: &mut Vec<ChunkArtifact>,
) -> Result<(), DocsError> {
    let work = &result.work;
    match &result.ocr {
        OcrImageResult::Text(text) => {
            outcome.pdf_image_ocr_runs += 1;
            emit_pdf_image_progress(
                document_id,
                work.current,
                work.total,
                &work.image,
                "ocr",
                "ok",
                &format!("bytes={}", text.len()),
            );
            outcome.warnings.push(format!(
                "PDF image OCR ok on page {} ({} bytes)",
                work.image.source_page,
                text.len()
            ));
            collected.extend(persist_image_ocr_chunks(
                db,
                document_id,
                work.image.source_page,
                &work.page_ids,
                text,
            )?);
        }
        OcrImageResult::NoText => emit_pdf_image_progress(
            document_id,
            work.current,
            work.total,
            &work.image,
            "ocr",
            "no-text",
            "",
        ),
        OcrImageResult::Failed(error) => {
            outcome.pdf_image_ocr_failures += 1;
            emit_pdf_image_progress(
                document_id,
                work.current,
                work.total,
                &work.image,
                "ocr",
                "failed",
                error,
            );
            outcome.warnings.push(format!(
                "PDF image OCR failed on page {}: {error}",
                work.image.source_page
            ));
        }
    }
    Ok(())
}

fn persist_vlm_result(
    db: &DbInstance,
    document_id: &str,
    work: &ImageWork,
    result: VlmImageResult,
    outcome: &mut PipelineOutcome,
    collected: &mut Vec<ChunkArtifact>,
) -> Result<(), DocsError> {
    match result {
        VlmImageResult::Described(description) => {
            collected.extend(persist_vlm_description(
                db,
                document_id,
                &work.page_ids,
                &description,
            )?);
            outcome.warnings.push(format!(
                "image description ok via {}/{} ({}ms, ${:.4})",
                description.provider,
                description.model,
                description.duration_ms,
                description.cost_usd
            ));
            outcome.vlm_descriptions += 1;
            emit_pdf_image_progress(
                document_id,
                work.current,
                work.total,
                &work.image,
                "vlm",
                "ok",
                "",
            );
        }
        VlmImageResult::Failed(error) => {
            outcome.pdf_image_vlm_failures += 1;
            outcome.warnings.push(error.clone());
            emit_pdf_image_progress(
                document_id,
                work.current,
                work.total,
                &work.image,
                "vlm",
                "failed",
                &error,
            );
        }
        VlmImageResult::Disabled(reason) => {
            outcome
                .warnings
                .push(format!("image description skipped: {reason}"));
            emit_vlm_skip(document_id, work, &reason);
        }
        VlmImageResult::NoProvider => {
            let warning = "image description skipped: VLM provider not configured";
            outcome.warnings.push(warning.into());
            emit_vlm_skip(document_id, work, warning);
        }
        VlmImageResult::Empty => {
            let warning = "image description skipped: provider returned empty description";
            outcome.warnings.push(warning.into());
            emit_vlm_skip(document_id, work, warning);
        }
        VlmImageResult::Fatal(error) => {
            emit_pdf_image_progress(
                document_id,
                work.current,
                work.total,
                &work.image,
                "vlm",
                "failed",
                &error.to_string(),
            );
            return Err(error);
        }
    }
    Ok(())
}

pub(super) fn emit_vlm_skip(document_id: &str, work: &ImageWork, warning: &str) {
    emit_pdf_image_progress(
        document_id,
        work.current,
        work.total,
        &work.image,
        "vlm",
        "skipped",
        warning,
    );
}

fn persist_image_ocr_chunks(
    db: &DbInstance,
    document_id: &str,
    source_page: u32,
    page_ids: &[String],
    text: &str,
) -> Result<Vec<ChunkArtifact>, DocsError> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let artifact_id = format!("pdf-image-ocr-{}", uuid::Uuid::new_v4());
    let chunks = image_ocr_chunks(db, document_id, source_page, text, &artifact_id)?;
    for chunk in &chunks {
        for page_id in page_ids {
            store::insert_provenance_edge(
                db,
                &make_edge(&chunk.chunk_id, page_id, ProvenanceEdgeType::ExtractedFrom),
            )
            .map_err(|e| DocsError::Storage {
                message: e.to_string(),
            })?;
        }
    }
    index_chunks_if_provider_available(db, &chunks);
    Ok(chunks)
}

fn image_ocr_chunks(
    db: &DbInstance,
    document_id: &str,
    source_page: u32,
    text: &str,
    artifact_id: &str,
) -> Result<Vec<ChunkArtifact>, DocsError> {
    let page_offsets = vec![PageOffset {
        page: source_page,
        char_start: 0,
        char_end: text.len(),
    }];
    persist_text_artifact_chunks(
        db,
        document_id,
        artifact_id,
        "pdf_image_ocr_text",
        text,
        &page_offsets,
        Some(artifact_id),
    )
}
