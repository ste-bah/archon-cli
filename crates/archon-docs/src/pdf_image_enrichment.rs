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
use crate::pdf::{PdfImage, PdfImageOrigin};
use crate::pdf_image_progress::{emit_pdf_image_progress, emit_pdf_progress};
use crate::pdf_image_vlm::{VlmImageResult, describe_image};
use crate::provenance::make_edge;
use crate::store;

#[allow(clippy::too_many_arguments)] // db + doc context + policy + mutable outcome are all needed
pub(crate) async fn enrich_pdf_images(
    db: &DbInstance,
    document_id: &str,
    images: &[PdfImage],
    page_count: u32,
    policy: &archon_policy::EffectivePolicy,
    page_ids_by_number: &BTreeMap<u32, String>,
    pages_by_number: &mut BTreeMap<u32, PageArtifact>,
    outcome: &mut PipelineOutcome,
    // Pre-resolved scanned-book verdict from the active detector (`Some` for coverage/union;
    // `None` → fall back to the in-memory aspect heuristic).
    scanned_override: Option<bool>,
    // Whether the doc produced a text seal (a text layer or Marker OCR owns the pages). A scanned
    // book WITH text content skips enrichment; one WITHOUT (image-only) must OCR its page scans.
    has_text_content: bool,
) -> Result<Vec<ChunkArtifact>, DocsError> {
    let total = images.len();
    // Image-OCR + VLM-description chunks, returned so the caller can fold them into chunks_root.
    let mut collected: Vec<ChunkArtifact> = Vec::new();
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

    // Scanned-book guard. Full-page-scan images ARE the pages, so how we treat them depends on
    // whether a text layer / Marker already owns those pages (`has_text_content`):
    //   scanned + text content  → OCR/VLM would duplicate + waste → skip enrichment entirely.
    //   scanned + NO text layer → the scans are the ONLY content → OCR them (below), but VLM is
    //                             useless on a page reproduction, so run OCR-only.
    //   not scanned             → born-digital figures → enrich normally (OCR + VLM per policy).
    // `scanned_override` carries the coverage/union verdict; else the in-memory aspect heuristic.
    let is_scanned = scanned_override.unwrap_or_else(|| is_scanned_page_images(images, page_count));
    if is_scanned && has_text_content {
        let scans = images
            .iter()
            .filter(|i| matches!(i.origin, PdfImageOrigin::Embedded { .. }))
            .count();
        emit_pdf_progress(format!(
            "PDF image enrichment: doc={document_id} SKIPPED {scans} full-page scan(s) — scanned book, text layer owns the pages"
        ));
        outcome.warnings.push(format!(
            "scanned-book: skipped enrichment of {scans} full-page scan image(s)"
        ));
        return Ok(collected);
    }
    // Image-only scanned book: OCR the page scans (the only content) but force VLM off — a full-page
    // scan is a page reproduction, not a discrete figure, so a VLM description adds no value and (on
    // a 100+ page book) is very expensive. Born-digital docs enrich with the policy unchanged.
    let effective_policy = if is_scanned {
        let mut p = policy.clone();
        p.docs.pdf.vlm_per_page_image = false;
        p
    } else {
        policy.clone()
    };
    let policy = &effective_policy;

    if image_workers(policy) <= 1 {
        for item in work_items {
            let result = process_image(document_id.to_string(), item, policy.clone()).await;
            persist_image_result(db, document_id, result, outcome, &mut collected)?;
        }
        return Ok(collected);
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
        if let Err(error) = persist_image_result(db, document_id, result, outcome, &mut collected) {
            tasks.abort_all();
            return Err(error);
        }
    }
    Ok(collected)
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

// ---- `--jobs auto`: derive the worker count from FREE VRAM ----

/// Marginal VRAM per concurrent VLM slot (MiB). The model weights are resident ONCE (the
/// ollama server shares them across requests — see [`VLM_MODEL_RESERVE_MB`]); each
/// *additional* in-flight request costs roughly its KV cache + activations + image tensors.
/// 2500 MiB is a conservative envelope for qwen2.5vl:7b processing a single page-figure image.
pub const VLM_SLOT_MB: u64 = 2500;

/// Resident weights (MiB) of the configured VLM (qwen2.5vl:7b, ~6.5 GB). We must reserve this
/// EXPLICITLY because the free-VRAM probe (`archon_accel::detect`) runs at ingest start, BEFORE
/// ollama lazy-loads the model on its first request (and ollama unloads again after
/// `keep_alive`). On a cold/idle card the probe therefore sees the weights' VRAM as "free"; if
/// we budgeted worker slots against that number we would recommend N slots that only fit once
/// the 6.5 GB model is NOT loaded — then the load happens and the card OOMs. Subtracting the
/// reserve up front makes the recommendation survive the lazy load. Tied to the VLM model: if
/// the configured model changes, this must track its weight footprint.
///
/// Trade-off (intentional, safe direction): if the model happens to be ALREADY resident when we
/// probe (a warm card, another job holding it under keep_alive), we double-count its weights and
/// under-recommend slightly. Under-parallelizing is safe; OOMing is not.
pub const VLM_MODEL_RESERVE_MB: u64 = 6500;

/// Safety margin (MiB) left free on the card: driver/display churn, allocator fragmentation,
/// and co-tenant processes growing under us mid-ingest. The probe is taken once at ingest
/// start, so the margin must absorb drift over the whole run.
pub const VLM_HEADROOM_MB: u64 = 2048;

/// Hard cap on unified-memory (Apple Silicon) hosts. GPU and CPU share one pool there, so
/// over-committing shows up as OS memory *pressure* (a soft, uncatchable slowdown/kill), not
/// a CUDA-style OOM we could detect and back off from. Stay conservative regardless of pool
/// size.
const UNIFIED_MEMORY_MAX_WORKERS: u64 = 2;

/// Derive the image-enrichment worker count for `--jobs auto` from an accelerator probe.
/// Driven by *free* VRAM, not card size — a 32 GB card with 139 MB free under co-tenancy
/// must run serial. Pure function of the report so it is unit-testable without hardware.
/// Always returns at least 1 (serial); never exceeds the enrichment engine's cap of 16
/// (the same cap `image_workers` applies to the policy value).
pub fn auto_image_workers(report: &archon_accel::AcceleratorReport) -> u32 {
    let Some(gpu) = report.best_gpu() else {
        // No CUDA/Metal device: the VLM is running on CPU (or a remote endpoint); parallel
        // slots would only contend for the same cores. Stay serial.
        return 1;
    };
    // Budget slots against what remains AFTER the model's resident weights (which the cold-card
    // probe counts as free — see VLM_MODEL_RESERVE_MB) and the safety margin are set aside.
    let usable = gpu
        .free_mb
        .saturating_sub(VLM_MODEL_RESERVE_MB + VLM_HEADROOM_MB);
    let n = (usable / VLM_SLOT_MB).clamp(1, 16);
    let n = if report.unified_memory {
        n.min(UNIFIED_MEMORY_MAX_WORKERS)
    } else {
        n
    };
    n as u32
}

/// Is this embedded image a full-page SCAN — **large AND page-shaped**? The aspect gate is what
/// separates a page scan from a large figure: a page's long/short side ratio is ~1.2–1.7 (Letter
/// 1.29, A4 1.41, US Legal 1.65, and taller book formats — e.g. the Uexküll scans measure ~1.58,
/// some crops up to ~1.61), and that range holds for either orientation. A large *square* diagram
/// or a *wide* chart is large but not page-shaped, so it is NOT counted as a scan (closing the
/// false positive of the size-only check). No page dimensions are available yet — a follow-up can
/// replace this proxy with a true coverage % via the page MediaBox (also removes the DPI coupling
/// in the size floor).
pub(crate) fn is_page_scale(img: &PdfImage) -> bool {
    if !matches!(img.origin, PdfImageOrigin::Embedded { .. }) || img.width == 0 || img.height == 0 {
        return false;
    }
    let large = img.width.min(img.height) >= 1000;
    let long = img.width.max(img.height) as f64;
    let short = img.width.min(img.height) as f64;
    let page_shaped = (1.2..=1.7).contains(&(long / short));
    large && page_shaped
}

/// Heuristic: do these embedded images look like a SELF-SCANNED book — most pages carry exactly one
/// full-page SCAN (see [`is_page_scale`])? Such images ARE the page scans (Marker already OCRs them
/// via the text layer), so enriching them duplicates OCR + wastes the VLM.
///
/// The signal is the DISTRIBUTION, not the raw count: a born-digital doc clusters figures (many
/// pages with zero, some with several), so its fraction of "one page-scan, nothing else" pages stays
/// low — the King&Salvo article has 17 clustered, non-page-shaped figures (~24% of pages), while a
/// 281 pp scanned Uexküll has one page-scan per page (100%). Threshold 70% cleanly separates them.
pub(crate) fn is_scanned_page_images(images: &[PdfImage], page_count: u32) -> bool {
    if page_count == 0 {
        return false;
    }
    // (page-scale count, total embedded count) per page.
    let mut per_page: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    for img in images {
        if !matches!(img.origin, PdfImageOrigin::Embedded { .. }) {
            continue;
        }
        let entry = per_page.entry(img.source_page).or_insert((0, 0));
        entry.1 += 1;
        if is_page_scale(img) {
            entry.0 += 1;
        }
    }
    // A "scanned page" carries exactly one embedded image and it is a page scan.
    let scanned_pages = per_page
        .values()
        .filter(|(page_scale, embedded)| *page_scale == 1 && *embedded == 1)
        .count();
    scanned_pages as f64 / page_count as f64 >= 0.70
}

#[cfg(test)]
mod scan_detection_tests {
    use super::*;

    fn embedded(page: u32, w: u32, h: u32) -> PdfImage {
        PdfImage {
            bytes: vec![],
            mime: "image/png",
            source_page: page,
            source_pages: vec![page],
            width: w,
            height: h,
            origin: PdfImageOrigin::Embedded { xobject_name: None },
        }
    }

    #[test]
    fn scanned_book_one_large_image_per_page_is_detected() {
        // 5 pages, one ~full-page scan each → scanned book.
        let imgs: Vec<_> = (1..=5).map(|p| embedded(p, 2000, 3000)).collect();
        assert!(is_scanned_page_images(&imgs, 5));
    }

    #[test]
    fn born_digital_clustered_figures_are_not_scans() {
        // 17 pages, figures clustered on a few pages (some pages multiple) → NOT a scanned book.
        let mut imgs = vec![embedded(5, 1200, 800), embedded(5, 900, 600)];
        imgs.push(embedded(8, 1200, 700));
        imgs.push(embedded(8, 800, 600));
        imgs.push(embedded(8, 800, 600));
        imgs.push(embedded(12, 1000, 800));
        imgs.push(embedded(12, 1000, 800));
        imgs.push(embedded(6, 1100, 700)); // a lone figure page
        assert!(!is_scanned_page_images(&imgs, 17));
    }

    #[test]
    fn per_page_small_icons_are_not_scans() {
        // One SMALL image per page (e.g. a header logo) is not a full-page scan.
        let imgs: Vec<_> = (1..=5).map(|p| embedded(p, 120, 60)).collect();
        assert!(!is_scanned_page_images(&imgs, 5));
    }

    #[test]
    fn empty_is_not_a_scan() {
        assert!(!is_scanned_page_images(&[], 0));
        assert!(!is_scanned_page_images(&[], 10));
    }

    // ---- aspect-ratio gate (adoption #1): large but non-page-shaped images are NOT scans ----

    #[test]
    fn large_square_figures_per_page_are_not_scans() {
        // One LARGE ~square diagram per page — size-only would false-positive; the aspect gate
        // (ratio 1.0 ∉ [1.2,1.6]) correctly rejects it as a page-scan.
        let imgs: Vec<_> = (1..=6).map(|p| embedded(p, 1500, 1500)).collect();
        assert!(imgs.iter().all(|i| !is_page_scale(i)));
        assert!(!is_scanned_page_images(&imgs, 6));
    }

    #[test]
    fn large_wide_charts_per_page_are_not_scans() {
        // One LARGE 16:9 chart per page (ratio 1.78) — not page-shaped → not a scan.
        let imgs: Vec<_> = (1..=6).map(|p| embedded(p, 1920, 1080)).collect();
        assert!(imgs.iter().all(|i| !is_page_scale(i)));
        assert!(!is_scanned_page_images(&imgs, 6));
    }

    #[test]
    fn page_shaped_large_image_is_page_scale() {
        // Letter/A4/book ratios in [1.2,1.7] + large → page-scale (either orientation).
        assert!(is_page_scale(&embedded(1, 1275, 1650))); // Letter portrait 1.29
        assert!(is_page_scale(&embedded(1, 1650, 1275))); // Letter landscape
        assert!(is_page_scale(&embedded(1, 1200, 1860))); // ~book 1.55
        assert!(is_page_scale(&embedded(1, 1303, 2041))); // real Uexküll scan 1.566
        assert!(is_page_scale(&embedded(1, 1270, 2049))); // Uexküll crop 1.613 (was missed at 1.6)
        assert!(!is_page_scale(&embedded(1, 900, 1400))); // page-shaped but too small
        assert!(!is_page_scale(&embedded(1, 1000, 2000))); // 2.0 too tall → figure, not page
    }

    #[test]
    fn uexkull_like_scans_are_detected() {
        // 20 pages, one book-format scan each (varied crops 1.56–1.61) → scanned book.
        let dims = [(1303u32, 2041u32), (1270, 2049), (1309, 2049), (1274, 2045)];
        let imgs: Vec<_> = (1..=20)
            .map(|p| {
                let (w, h) = dims[(p as usize) % dims.len()];
                embedded(p, w, h)
            })
            .collect();
        assert!(is_scanned_page_images(&imgs, 20));
    }
}

#[cfg(test)]
mod auto_workers_tests {
    use super::*;
    use archon_accel::{AccelKind, Accelerator, AcceleratorReport};

    fn report(accelerators: Vec<Accelerator>, unified_memory: bool) -> AcceleratorReport {
        AcceleratorReport {
            platform: "test".into(),
            arch: "test".into(),
            accelerators,
            host_ram_total_mb: 32768,
            host_ram_free_mb: 16384,
            unified_memory,
            notes: vec![],
        }
    }

    fn gpu(kind: AccelKind, total_mb: u64, free_mb: u64) -> Accelerator {
        Accelerator {
            kind,
            index: 0,
            name: "test-gpu".into(),
            total_mb,
            free_mb,
        }
    }

    #[test]
    fn no_gpu_is_serial() {
        // CPU-only host (or only a Cpu accelerator entry): the VLM has no card to pack; serial.
        assert_eq!(auto_image_workers(&report(vec![], false)), 1);
        assert_eq!(
            auto_image_workers(&report(vec![gpu(AccelKind::Cpu, 32768, 16384)], false)),
            1
        );
    }

    #[test]
    fn co_tenancy_starved_card_is_serial() {
        // The 5090 co-tenancy case: 32 GB card with 139 MB free → free-driven math floors at 1.
        let r = report(vec![gpu(AccelKind::Cuda, 32768, 139)], false);
        assert_eq!(auto_image_workers(&r), 1);
    }

    #[test]
    fn laptop_8gb_cold_card_is_serial() {
        // RTX 5070 laptop-class, COLD card: probe sees ~8192 MB free, but the 6.5 GB VLM
        // weights are NOT loaded yet. Budgeting slots against 8192 and THEN loading the model
        // would OOM. Reserving weights+headroom: 8192.saturating_sub(6500+2048)=0 → N=1 (SAFE).
        let r = report(vec![gpu(AccelKind::Cuda, 8192, 8192)], false);
        assert_eq!(auto_image_workers(&r), 1);
    }

    #[test]
    fn laptop_8gb_post_marker_is_serial() {
        // Realistic mid-run 8 GB state (Marker/other tenants already resident, ~1.5 GB free):
        // well under the model reserve → serial.
        let r = report(vec![gpu(AccelKind::Cuda, 8192, 1500)], false);
        assert_eq!(auto_image_workers(&r), 1);
    }

    #[test]
    fn thirty_gb_free_scales_up_and_huge_card_caps_at_16() {
        // 5090-class, 29887 MB free → (29887 - 6500 - 2048) / 2500 = 8 workers.
        let r = report(vec![gpu(AccelKind::Cuda, 32768, 29887)], false);
        assert_eq!(auto_image_workers(&r), 8);
        // 64 GB free → (65536 - 8548) / 2500 = 22, clamped to the engine's cap of 16.
        let r = report(vec![gpu(AccelKind::Cuda, 65536, 65536)], false);
        assert_eq!(auto_image_workers(&r), 16);
    }

    #[test]
    fn unified_memory_caps_at_two() {
        // Mac 24 GB unified, 20480 MB free → (20480 - 8548) / 2500 = 4 raw, but memory
        // pressure is uncatchable there — hard cap 2.
        let r = report(vec![gpu(AccelKind::Metal, 24576, 20480)], true);
        assert_eq!(auto_image_workers(&r), 2);
        // Unified but tiny free pool still floors at 1, not 2.
        let r = report(vec![gpu(AccelKind::Metal, 8192, 1024)], true);
        assert_eq!(auto_image_workers(&r), 1);
    }
}
