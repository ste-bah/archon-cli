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
    // Captured when the text root is sealed, so image-OCR/VLM chunks can be re-folded into it
    // after enrichment (V-1 image integrity).
    struct TextSeal {
        ocr_artifact_id: String,
        text_chunks: Vec<crate::models::ChunkArtifact>,
        spatial_by: std::collections::BTreeMap<String, String>,
        ocr_engine: &'static str,
        source_sha256: String,
    }
    let mut text_seal: Option<TextSeal> = None;
    if !extract_result.full_text.trim().is_empty() {
        let ocr_artifact_id = format!("ocr-result-{}", ocr_run_id);
        let (chunks, spatials, ocr_engine) = if policy.docs.pdf.use_token_aware_chunker() {
            // Token-aware path (best-system default). Prefer Marker blocks (real bboxes,
            // device-agnostic sidecar) when a sidecar is configured; otherwise — or on any
            // Marker failure — fall back to blocks synthesized from the flat extracted text
            // (token-budgeted chunking, page lineage preserved, no bboxes).
            let text_blocks = || {
                crate::block_chunking::blocks_from_text(
                    &extract_result.full_text,
                    &extract_result.page_offsets,
                )
            };
            let (blocks, coord) = match crate::marker_source::from_policy(
                &policy.docs.pdf,
                extract_result.page_count,
            ) {
                Some(src) => match src.blocks_for(Path::new(file_path)).await {
                    Ok(b) if !b.is_empty() => (b, crate::block_chunking::COORD_MARKER),
                    Ok(_) => {
                        outcome.warnings.push(
                            "marker returned no blocks; falling back to text chunking".to_string(),
                        );
                        (text_blocks(), crate::block_chunking::COORD_NONE)
                    }
                    Err(e) => {
                        outcome.warnings.push(format!(
                            "marker sidecar failed ({e}); falling back to text chunking"
                        ));
                        (text_blocks(), crate::block_chunking::COORD_NONE)
                    }
                },
                None => (text_blocks(), crate::block_chunking::COORD_NONE),
            };
            let ocr_engine = if coord == crate::block_chunking::COORD_MARKER {
                "marker"
            } else {
                "poppler"
            };
            let (chunks, spatials) = crate::block_chunking::persist_block_chunks(
                db,
                document_id,
                &ocr_artifact_id,
                "ocr_text",
                &blocks,
                coord,
            )?;
            (chunks, spatials, ocr_engine)
        } else {
            let chunks = persist_text_artifact_chunks(
                db,
                document_id,
                &ocr_artifact_id,
                "ocr_text",
                &extract_result.full_text,
                &extract_result.page_offsets,
                None,
            )?;
            (chunks, Vec::new(), "poppler")
        };
        let edges = build_doc_lineage_edges(document_id, &ocr_artifact_id, &chunks, &page_ids);
        for edge in &edges {
            store::insert_provenance_edge(db, edge).map_err(|e| DocsError::Storage {
                message: e.to_string(),
            })?;
        }
        // V-1: per-chunk integrity hashes + chunks_root provenance record (all ingest, both
        // chunkers) — gives every chunk tamper-evidence and fills the artifact's previously
        // empty provenance_record_id.
        if !chunks.is_empty() {
            let mut spatial_by = std::collections::BTreeMap::new();
            for s in &spatials {
                spatial_by.insert(s.chunk_id.clone(), s.spatial_hash.clone());
            }
            // Provenance input = the true source-FILE hash (doc_sources.content_hash), not the
            // extracted text — so the extract_text_spatial record's input_hashes name the input.
            let source_sha256 = store::get_doc_source(db, document_id)
                .ok()
                .flatten()
                .map(|d| d.content_hash)
                .unwrap_or_default();
            crate::provenance_chunks::persist_chunk_integrity(
                db,
                &ocr_artifact_id,
                &chunks,
                &spatial_by,
                ocr_engine,
                &source_sha256,
                ocr_run_id,
            )?;
            text_seal = Some(TextSeal {
                ocr_artifact_id: ocr_artifact_id.clone(),
                text_chunks: chunks.clone(),
                spatial_by,
                ocr_engine,
                source_sha256,
            });
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
    // Active scanned-book detector governs whether page-scan images are enriched. For the
    // coverage/union detectors we resolve the verdict from the path here (cheap: pdfimages + lopdf,
    // milliseconds) and pass the SAME selected verdict the pre-ingest report shows — including its
    // aspect fallback when page dims are unreadable — so report and pipeline never disagree. Aspect
    // mode (the default) → None, so enrich_pdf_images keeps using its in-memory heuristic, unchanged.
    let scanned_override = {
        let detector = crate::pdf_scan::ScanDetector::parse(&policy.docs.pdf.scan_detector);
        match detector {
            crate::pdf_scan::ScanDetector::Aspect => None,
            _ => Some(
                crate::pdf_scan::classify_scan(Path::new(file_path), detector, &policy.docs.pdf)
                    .active_scanned,
            ),
        }
    };
    let image_chunks = enrich_pdf_images(
        db,
        document_id,
        &pdf_images,
        extract_result.page_count,
        policy,
        &page_ids_by_number,
        &mut pages_by_number,
        &mut outcome,
        scanned_override,
    )
    .await?;
    // V-1 image integrity: image-OCR + VLM-description chunks are persisted AFTER the early text
    // seal, so re-fold them into chunks_root — re-run persist_chunk_integrity over the text+image
    // UNION (idempotent upsert; chunks_root sorts commits, so text-only ingests stay byte-identical).
    // Only fires when the doc had a text seal AND produced image chunks. (Image-only/scanned PDFs
    // with no text seal still get no root — a documented follow-up needing a synthetic artifact.)
    if let Some(seal) = text_seal
        && !image_chunks.is_empty()
    {
        let mut all = seal.text_chunks;
        all.extend(image_chunks);
        crate::provenance_chunks::persist_chunk_integrity(
            db,
            &seal.ocr_artifact_id,
            &all,
            &seal.spatial_by,
            seal.ocr_engine,
            &seal.source_sha256,
            ocr_run_id,
        )?;
    }
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
