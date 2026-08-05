//! Persistence for enriched PDF images: OCR runs, VLM descriptions, and the derived
//! OCR chunks folded into the document's chunks_root.
//!
//! Split out of `pdf_image_enrichment.rs` to keep both files under the 500-line gate.

use super::*;

/// S5 (index-overhaul): write the durable per-image OCR outcome to `doc_image_ocr_status`.
/// A write failure is surfaced as a warning — the enrichment must not die on a
/// bookkeeping row — but the row itself turns "image text missing" from a log line
/// into a queryable degradation state.
fn persist_ocr_status(
    db: &DbInstance,
    document_id: &str,
    work: &ImageWork,
    status: &str,
    detail: &str,
    outcome: &mut PipelineOutcome,
) {
    let status_id = format!(
        "{document_id}-p{}-i{}",
        work.image.source_page, work.current
    );
    let mut params = std::collections::BTreeMap::new();
    params.insert(
        "rows".to_string(),
        cozo::DataValue::List(vec![cozo::DataValue::List(vec![
            cozo::DataValue::from(status_id.as_str()),
            cozo::DataValue::from(document_id),
            cozo::DataValue::from(work.image.source_page as i64),
            cozo::DataValue::from(status),
            cozo::DataValue::from(&detail[..detail.len().min(400)]),
            cozo::DataValue::from(chrono::Utc::now().to_rfc3339().as_str()),
        ])]),
    );
    if let Err(e) = crate::cozo_retry::run_script_guarded(
        db,
        "?[status_id, document_id, page_number, status, detail, created_at] <- $rows \
         :put doc_image_ocr_status { status_id => document_id, page_number, status, \
         detail, created_at }",
        params,
        cozo::ScriptMutability::Mutable,
        "put doc_image_ocr_status",
    ) {
        outcome.warnings.push(format!(
            "image OCR status row write failed ({status_id}): {e}"
        ));
    }
}

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
        OcrImageResult::Text { text, quality } => {
            // S8: the status row records which engine won and how the text scored;
            // a successful-but-low-quality result is `suspect`, not `ok`.
            let (status, detail) = match quality {
                Some(meta) => {
                    let status = if meta.score < crate::ocr::quality::quality_floor() {
                        "suspect"
                    } else {
                        "ok"
                    };
                    let mut detail = format!("engine={} score={:.2}", meta.engine, meta.score);
                    if meta.escalated {
                        detail.push_str(" escalated");
                    }
                    if let Some(note) = &meta.note {
                        detail.push_str(&format!(" ({note})"));
                    }
                    (status, detail)
                }
                None => ("ok", String::new()),
            };
            persist_ocr_status(db, document_id, work, status, &detail, outcome);
            outcome.pdf_image_ocr_runs += 1;
            emit_pdf_image_progress(
                document_id,
                work.current,
                work.total,
                &work.image,
                "ocr",
                status,
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
        OcrImageResult::NoText => {
            persist_ocr_status(db, document_id, work, "no-text", "", outcome);
            emit_pdf_image_progress(
                document_id,
                work.current,
                work.total,
                &work.image,
                "ocr",
                "no-text",
                "",
            );
        }
        OcrImageResult::Failed(error) => {
            persist_ocr_status(db, document_id, work, "failed", error, outcome);
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
