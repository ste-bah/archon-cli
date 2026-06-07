use std::collections::BTreeMap;
use std::fs;

use cozo::DbInstance;
use tokio::task::JoinSet;

use crate::errors::DocsError;
use crate::hash::sha256_hex;
use crate::ingest::PipelineOutcome;
use crate::ingest_artifacts::{index_chunks_if_provider_available, persist_text_artifact_chunks};
use crate::ingest_multimodal::persist_vlm_description;
use crate::models::{ChunkArtifact, PageArtifact, PageOffset, ProvenanceEdgeType};
use crate::ocr::local::LocalOcrProvider;
use crate::ocr::provider::{self as ocr_provider, OcrProvider, OcrRequest};
use crate::pdf::PdfImage;
use crate::pdf_image_progress::{emit_pdf_image_progress, emit_pdf_progress};
use crate::pdf_image_vlm::{VlmImageResult, describe_image};
use crate::provenance::make_edge;
use crate::store;

pub(crate) async fn enrich_pdf_images(
    db: &DbInstance,
    document_id: &str,
    images: &[PdfImage],
    policy: &archon_policy::EffectivePolicy,
    page_ids_by_number: &BTreeMap<u32, String>,
    pages_by_number: &mut BTreeMap<u32, PageArtifact>,
    outcome: &mut PipelineOutcome,
) -> Result<(), DocsError> {
    let total = images.len();
    emit_pdf_progress(format!(
        "PDF image enrichment: doc={document_id} images={total} ocr=enabled vlm={} provider={} workers={}",
        policy.docs.pdf.vlm_per_page_image,
        policy.docs.vlm.provider,
        image_workers(policy)
    ));

    let mut work_items = Vec::new();
    for (index, image) in images.iter().enumerate() {
        let current = index + 1;
        mark_page_image_metadata(db, pages_by_number, image)?;
        let page_ids = image
            .source_pages
            .iter()
            .filter_map(|page| page_ids_by_number.get(page).cloned())
            .collect::<Vec<_>>();
        if page_ids.is_empty() {
            emit_pdf_image_progress(
                document_id,
                current,
                total,
                image,
                "page-link",
                "skipped",
                "no page artifact exists",
            );
            outcome.warnings.push(format!(
                "PDF image on page {} skipped: no page artifact exists",
                image.source_page
            ));
            continue;
        }
        work_items.push(ImageWork {
            current,
            total,
            image: image.clone(),
            page_ids,
        });
    }

    if image_workers(policy) <= 1 {
        for item in work_items {
            let result = process_image(document_id.to_string(), item, policy.clone()).await;
            persist_image_result(db, document_id, result, outcome)?;
        }
        return Ok(());
    }

    let mut next = 0usize;
    let mut tasks = JoinSet::new();
    let workers = image_workers(policy).min(work_items.len().max(1));
    while next < work_items.len() || !tasks.is_empty() {
        while next < work_items.len() && tasks.len() < workers {
            let item = work_items[next].clone();
            let policy = policy.clone();
            let doc = document_id.to_string();
            tasks.spawn(async move { process_image(doc, item, policy).await });
            next += 1;
        }
        let Some(joined) = tasks.join_next().await else {
            continue;
        };
        let result = joined.map_err(|e| DocsError::VlmProvider {
            provider: "runtime".into(),
            message: format!("PDF image worker join failed: {e}"),
            status_code: None,
        })?;
        if let Err(error) = persist_image_result(db, document_id, result, outcome) {
            tasks.abort_all();
            return Err(error);
        }
    }
    Ok(())
}

#[derive(Clone)]
struct ImageWork {
    current: usize,
    total: usize,
    image: PdfImage,
    page_ids: Vec<String>,
}

struct ImageResult {
    work: ImageWork,
    ocr: OcrImageResult,
    vlm: Option<VlmImageResult>,
}

enum OcrImageResult {
    Text(String),
    NoText,
    Failed(String),
}

async fn process_image(
    document_id: String,
    work: ImageWork,
    policy: archon_policy::EffectivePolicy,
) -> ImageResult {
    emit_pdf_image_progress(
        &document_id,
        work.current,
        work.total,
        &work.image,
        "ocr",
        "start",
        "",
    );
    let ocr = match extract_image_ocr_text(&work.image).await {
        Ok(Some(text)) => OcrImageResult::Text(text),
        Ok(None) => OcrImageResult::NoText,
        Err(error) => OcrImageResult::Failed(error.to_string()),
    };

    let vlm = if policy.docs.pdf.vlm_per_page_image {
        emit_pdf_image_progress(
            &document_id,
            work.current,
            work.total,
            &work.image,
            "vlm",
            "start",
            &policy.docs.vlm.provider,
        );
        Some(describe_image(policy, work.image.bytes.clone()).await)
    } else {
        None
    };

    ImageResult { work, ocr, vlm }
}

fn persist_image_result(
    db: &DbInstance,
    document_id: &str,
    result: ImageResult,
    outcome: &mut PipelineOutcome,
) -> Result<(), DocsError> {
    persist_ocr_result(db, document_id, &result, outcome)?;
    if let Some(vlm) = result.vlm {
        persist_vlm_result(db, document_id, &result.work, vlm, outcome)?;
    }
    Ok(())
}

fn persist_ocr_result(
    db: &DbInstance,
    document_id: &str,
    result: &ImageResult,
    outcome: &mut PipelineOutcome,
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
            persist_image_ocr_chunks(
                db,
                document_id,
                work.image.source_page,
                &work.page_ids,
                text,
            )?;
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
) -> Result<(), DocsError> {
    match result {
        VlmImageResult::Described(description) => {
            persist_vlm_description(db, document_id, &work.page_ids, &description)?;
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

fn emit_vlm_skip(document_id: &str, work: &ImageWork, warning: &str) {
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
) -> Result<(), DocsError> {
    if text.trim().is_empty() {
        return Ok(());
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
    Ok(())
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

async fn extract_image_ocr_text(image: &PdfImage) -> Result<Option<String>, DocsError> {
    let ext = match image.mime {
        "image/jpeg" => "jpg",
        _ => "png",
    };
    let dir = std::env::temp_dir().join(format!("archon-pdf-image-ocr-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("image.{ext}"));
    fs::write(&path, &image.bytes)?;
    let local_provider = LocalOcrProvider;
    let configured_provider = ocr_provider::get_provider();
    let provider: &dyn OcrProvider = configured_provider.as_deref().unwrap_or(&local_provider);
    let result = provider
        .extract(OcrRequest {
            file_path: path.to_string_lossy().to_string(),
            document_id: "pdf-image".into(),
            ocr_run_id: format!("ocr-image-{}", uuid::Uuid::new_v4()),
            page_range: None,
            language_hint: None,
        })
        .await;
    let _ = fs::remove_dir_all(&dir);
    result.map(|ocr| {
        if ocr.full_text.trim().is_empty() {
            None
        } else {
            Some(ocr.full_text)
        }
    })
}

fn mark_page_image_metadata(
    db: &DbInstance,
    pages_by_number: &mut BTreeMap<u32, PageArtifact>,
    image: &PdfImage,
) -> Result<(), DocsError> {
    let image_hash = sha256_hex(&image.bytes);
    for page_number in &image.source_pages {
        if let Some(page) = pages_by_number.get_mut(page_number)
            && page.image_hash.is_none()
        {
            page.image_hash = Some(image_hash.clone());
            if image.width > 0 {
                page.width = Some(image.width as f32);
            }
            if image.height > 0 {
                page.height = Some(image.height as f32);
            }
            store::insert_page(db, page).map_err(|e| DocsError::Storage {
                message: e.to_string(),
            })?;
        }
    }
    Ok(())
}

fn image_workers(policy: &archon_policy::EffectivePolicy) -> usize {
    policy.docs.pdf.image_enrichment_workers.clamp(1, 16) as usize
}
