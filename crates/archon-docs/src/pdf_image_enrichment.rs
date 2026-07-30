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
            match tokio::time::timeout(
                per_image_timeout(),
                process_image(document_id.to_string(), item.clone(), policy.clone()),
            )
            .await
            {
                Ok(result) => {
                    persist_image_result(db, document_id, result, outcome, &mut collected)?
                }
                Err(_elapsed) => record_image_timeout(document_id, &item, outcome),
            }
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
            // On timeout, `process_image` never returns — hand the work back so the caller can
            // synthesize a per-image skip (JoinSet gives no task identity at join time).
            tasks.spawn(async move {
                let work = item.clone();
                match tokio::time::timeout(per_image_timeout(), process_image(doc, item, policy))
                    .await
                {
                    Ok(result) => Ok(result),
                    Err(_elapsed) => Err(work),
                }
            });
            next += 1;
        }
        let Some(joined) = tasks.join_next().await else {
            // Defensive: an emptied set with no work left must exit, never spin.
            if next >= work_items.len() {
                break;
            }
            continue;
        };
        let result = joined.map_err(|e| DocsError::VlmProvider {
            provider: "runtime".into(),
            message: format!("PDF image worker join failed: {e}"),
            status_code: None,
        })?;
        match result {
            Ok(result) => {
                if let Err(error) =
                    persist_image_result(db, document_id, result, outcome, &mut collected)
                {
                    tasks.abort_all();
                    return Err(error);
                }
            }
            // A wall-clock timeout skips just this image; the run continues.
            Err(work) => record_image_timeout(document_id, &work, outcome),
        }
    }
    Ok(collected)
}

/// Per-image wall-clock backstop over the WHOLE `process_image` (OCR + VLM) of one image. A
/// LOOSE budget by design: the default 600s sits well above legitimate worst-case VLM time
/// (the 120s per-request reqwest timeout × retries + backoff), so it only fires on a truly
/// wedged external call (the observed 0-CPU ingest hang), never on slow-but-progressing work.
fn per_image_timeout() -> std::time::Duration {
    let secs = std::env::var("ARCHON_PDF_IMAGE_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(600);
    std::time::Duration::from_secs(secs)
}

/// The per-image backstop fired: record a per-image SKIP (progress line + failure counter +
/// warning) and move on — a hung image must never be fatal to the document.
fn record_image_timeout(document_id: &str, work: &ImageWork, outcome: &mut PipelineOutcome) {
    let warning = format!(
        "PDF image enrichment timed out after {}s on page {} (image skipped)",
        per_image_timeout().as_secs(),
        work.image.source_page
    );
    outcome.pdf_image_vlm_failures += 1;
    outcome.warnings.push(warning.clone());
    emit_vlm_skip(document_id, work, &warning);
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

#[path = "pdf_image_persist.rs"]
mod persist;
use persist::{emit_vlm_skip, persist_image_result};

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
#[path = "pdf_image_enrichment_tests.rs"]
mod tests;
