use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

use archon_policy::PdfPolicy;
use tokio::process::Command;

use crate::errors::DocsError;
use crate::hash::sha256_hex;
use crate::models::PageOffset;
use crate::tool_path::command_path;

#[derive(Clone, Debug)]
pub struct PdfExtractResult {
    pub full_text: String,
    pub page_count: u32,
    pub page_offsets: Vec<PageOffset>,
    pub embedded_images: Vec<PdfImage>,
    pub rendered_pages: Vec<PdfImage>,
    pub embedded_images_skipped_filter: usize,
    pub warnings: Vec<String>,
    pub processing_duration_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfImage {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub source_page: u32,
    pub source_pages: Vec<u32>,
    pub width: u32,
    pub height: u32,
    pub origin: PdfImageOrigin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PdfImageOrigin {
    Embedded { xobject_name: Option<String> },
    RenderedPage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfImagesListEntry {
    pub source_page: u32,
    pub source_pages: Vec<u32>,
    pub width: u32,
    pub height: u32,
    pub object_key: Option<String>,
    pub xobject_name: Option<String>,
    /// Effective horizontal resolution AS DRAWN (`x-ppi` column of `pdfimages -list`). `None` when
    /// the column is absent/unparsable; `0`/`1` for JBIG2/CCITT images poppler can't rate. Used by
    /// the coverage classifier ([`crate::pdf_scan`]) to recover the drawn size in points.
    pub x_ppi: Option<u32>,
    /// Effective vertical resolution as drawn (`y-ppi` column). See [`Self::x_ppi`].
    pub y_ppi: Option<u32>,
    /// In-PDF (compressed) byte size from the `size` column (e.g. `479K`, `1620B`). `None` when
    /// absent/unparsable. A close proxy for the extracted-PNG size that the pipeline's
    /// `min_image_bytes` filter uses, so the pre-ingest classifier can approximate that filter
    /// without extracting bytes.
    pub bytes: Option<u64>,
}

/// A lightweight pre-ingest classification of how the pipeline will treat a PDF's images —
/// computed from `pdfimages -list` (dims + ppi), `pdfinfo` (page count), and `lopdf` (page dims)
/// ONLY, with no byte extraction and no Marker. Lets the CLI show a loud report + confirm before
/// committing to the ingest. Carries BOTH detector verdicts (aspect + coverage) so the A/B
/// comparison is visible before any OCR/VLM runs.
#[derive(Debug, Clone)]
pub struct EnrichmentClassification {
    pub page_count: u32,
    pub embedded_images: usize,
    /// Page-scan count under the ACTIVE detector (selected by policy `scan_detector`).
    pub page_scans: usize,
    /// ACTIVE verdict — what the ingest pipeline will actually do.
    pub is_scanned_book: bool,
    /// Images that WILL be enriched (OCR + VLM): 0 for a scanned book, else all embedded images.
    pub will_enrich: usize,
    /// Which detector produced the active verdict: `"aspect"` | `"coverage"`.
    pub detector: &'static str,
    /// A/B: aspect-heuristic verdict + its page-scan count (always computed).
    pub aspect_scanned: bool,
    pub aspect_page_scans: usize,
    /// A/B: coverage verdict + page-scan count + peak page coverage. `None` when page dims were
    /// unavailable (no `lopdf` parse / no MediaBox in ancestry) so coverage could not be computed.
    pub coverage_scanned: Option<bool>,
    pub coverage_page_scans: Option<usize>,
    pub coverage_max: Option<f64>,
    /// Coverage produced a verdict but deferred ≥1 image to aspect (unusable ppi / missing page
    /// dims) — the coverage verdict is advisory; the report flags the doc for review.
    pub coverage_low_confidence: bool,
    /// The two detectors disagree on the scanned/born-digital verdict.
    pub divergent: bool,
    /// Whether `pdftotext` finds a usable text layer. A scanned book WITH a text layer skips image
    /// enrichment (Marker/text owns the pages); one WITHOUT (image-only) OCRs its page scans for
    /// content, so the report must not claim "skipped" for those.
    pub has_text_layer: bool,
}

/// Classify a PDF for image enrichment without extracting image bytes or running Marker. Runs BOTH
/// the aspect heuristic (the shipped detector) and the coverage classifier
/// ([`crate::pdf_scan`]), selecting the active verdict from `pdf_policy.scan_detector`, so the
/// pre-ingest report matches what the pipeline will actually do and surfaces any A/B divergence.
pub fn classify_pdf_enrichment(path: &Path, pdf_policy: &PdfPolicy) -> EnrichmentClassification {
    let detector = crate::pdf_scan::ScanDetector::parse(&pdf_policy.scan_detector);
    let scan = crate::pdf_scan::classify_scan(path, detector, pdf_policy);
    let embedded_images = scan.embedded_images;
    let is_scanned_book = scan.active_scanned;
    EnrichmentClassification {
        page_count: scan.page_count,
        embedded_images,
        page_scans: scan.active_page_scans,
        is_scanned_book,
        will_enrich: if is_scanned_book { 0 } else { embedded_images },
        detector: scan.detector.as_str(),
        aspect_scanned: scan.aspect_scanned,
        aspect_page_scans: scan.aspect_page_scans,
        coverage_scanned: scan.coverage.as_ref().map(|c| c.scanned),
        coverage_page_scans: scan.coverage.as_ref().map(|c| c.page_scans),
        coverage_max: scan.coverage.as_ref().map(|c| c.max_coverage),
        coverage_low_confidence: scan
            .coverage
            .as_ref()
            .map(|c| c.low_confidence)
            .unwrap_or(false),
        divergent: scan.divergent,
        has_text_layer: pdf_has_text_layer(path),
    }
}

/// Quick pre-ingest probe: does `pdftotext` extract a non-trivial text layer? Used only to make the
/// enrichment report honest about image-only scans (which get OCR'd, not skipped). A handful of
/// characters (stray artifacts on an otherwise image-only scan) do not count as a text layer.
fn pdf_has_text_layer(path: &Path) -> bool {
    let Ok(output) = std::process::Command::new(command_path("pdftotext", "ARCHON_PDFTOTEXT_BIN"))
        .arg("-layout")
        .arg(path)
        .arg("-")
        .output()
    else {
        return false;
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .chars()
            .filter(|c| !c.is_whitespace())
            .count()
            >= 16
}

pub(crate) fn pdf_page_count(path: &Path) -> Option<u32> {
    let output = std::process::Command::new(command_path("pdfinfo", "ARCHON_PDFINFO_BIN"))
        .arg(path)
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("Pages:")?.trim().parse::<u32>().ok())
}

pub(crate) fn list_embedded_image_dims(path: &Path) -> Vec<PdfImagesListEntry> {
    match std::process::Command::new(command_path("pdfimages", "ARCHON_PDFIMAGES_BIN"))
        .arg("-list")
        .arg(path)
        .output()
    {
        Ok(o) if o.status.success() => parse_pdfimages_list(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

pub async fn extract_pdf_unified(
    path: &Path,
    pdf_policy: &PdfPolicy,
) -> Result<PdfExtractResult, DocsError> {
    let started = Instant::now();
    let mut warnings = Vec::new();
    let text_result = extract_text_layer(path).await;
    let text_error = text_result.as_ref().err().map(ToString::to_string);
    let full_text = match text_result {
        Ok(text) => text,
        Err(e) => {
            warnings.push(format!("pdftotext skipped: {e}"));
            String::new()
        }
    };

    let mut embedded_images = Vec::new();
    let mut embedded_images_skipped_filter = 0usize;
    if pdf_policy.extract_embedded_images {
        match extract_embedded_images(path, pdf_policy).await {
            Ok(result) => {
                embedded_images = result.images;
                embedded_images_skipped_filter = result.skipped_filter;
                warnings.extend(result.warnings);
            }
            Err(e) => warnings.push(format!("embedded image extraction skipped: {e}")),
        }
    }

    let should_render_pages = pdf_policy.render_text_pdf_pages
        || (full_text.trim().is_empty() && embedded_images.is_empty());
    let mut rendered_pages = Vec::new();
    if should_render_pages {
        match render_pdf_pages(path).await {
            Ok(images) => rendered_pages = images,
            Err(e) if full_text.trim().is_empty() && embedded_images.is_empty() => {
                return Err(match text_error {
                    Some(text_error) => DocsError::OcrApi {
                        message: format!("{text_error}; PDF render fallback also failed: {e}"),
                        status_code: None,
                    },
                    None => e,
                });
            }
            Err(e) => warnings.push(format!("PDF render fallback skipped: {e}")),
        }
    }

    let mut page_offsets = if full_text.trim().is_empty() {
        Vec::new()
    } else {
        compute_pdf_page_offsets(&full_text)
    };
    let max_image_page = embedded_images
        .iter()
        .chain(rendered_pages.iter())
        .flat_map(|image| image.source_pages.iter().copied())
        .max()
        .unwrap_or(0);
    let page_count = page_offsets.len().max(max_image_page as usize).max(1) as u32;
    if page_offsets.is_empty() {
        page_offsets = empty_page_offsets(page_count);
    } else {
        let existing = page_offsets.len() as u32;
        for page in (existing + 1)..=page_count {
            page_offsets.push(PageOffset {
                page,
                char_start: 0,
                char_end: 0,
            });
        }
    }

    Ok(PdfExtractResult {
        full_text,
        page_count,
        page_offsets,
        embedded_images,
        rendered_pages,
        embedded_images_skipped_filter,
        warnings,
        processing_duration_ms: started.elapsed().as_millis() as u64,
    })
}

async fn extract_text_layer(path: &Path) -> Result<String, DocsError> {
    let output = Command::new(command_path("pdftotext", "ARCHON_PDFTOTEXT_BIN"))
        .arg("-layout")
        .arg(path)
        .arg("-")
        .output()
        .await
        .map_err(|e| DocsError::OcrApi {
            message: format!("pdftotext not found. Install poppler-utils. ({e})"),
            status_code: None,
        })?;
    if !output.status.success() {
        return Err(DocsError::OcrApi {
            message: format!(
                "pdftotext failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            status_code: output.status.code().map(|code| code as u16),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

struct EmbeddedImageExtraction {
    images: Vec<PdfImage>,
    skipped_filter: usize,
    warnings: Vec<String>,
}

async fn extract_embedded_images(
    path: &Path,
    pdf_policy: &PdfPolicy,
) -> Result<EmbeddedImageExtraction, DocsError> {
    let list_output = Command::new(command_path("pdfimages", "ARCHON_PDFIMAGES_BIN"))
        .arg("-list")
        .arg(path)
        .output()
        .await
        .map_err(|e| DocsError::OcrApi {
            message: format!("pdfimages not found. Install poppler-utils. ({e})"),
            status_code: None,
        })?;
    if !list_output.status.success() {
        return Err(DocsError::OcrApi {
            message: format!(
                "pdfimages -list failed: {}",
                String::from_utf8_lossy(&list_output.stderr).trim()
            ),
            status_code: list_output.status.code().map(|code| code as u16),
        });
    }
    let entries = parse_pdfimages_list(&String::from_utf8_lossy(&list_output.stdout));
    if entries.is_empty() {
        return Ok(EmbeddedImageExtraction {
            images: Vec::new(),
            skipped_filter: 0,
            warnings: Vec::new(),
        });
    }

    let extract_dir =
        std::env::temp_dir().join(format!("archon-pdf-images-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&extract_dir)?;
    let prefix = extract_dir.join("img");
    let extract_output = Command::new(command_path("pdfimages", "ARCHON_PDFIMAGES_BIN"))
        .arg("-png")
        .arg(path)
        .arg(&prefix)
        .output()
        .await;
    let extract_output = match extract_output {
        Ok(output) => output,
        Err(e) => {
            let _ = fs::remove_dir_all(&extract_dir);
            return Err(DocsError::OcrApi {
                message: format!("pdfimages failed to start extraction: {e}"),
                status_code: None,
            });
        }
    };
    if !extract_output.status.success() {
        let stderr = String::from_utf8_lossy(&extract_output.stderr);
        let _ = fs::remove_dir_all(&extract_dir);
        return Err(DocsError::OcrApi {
            message: format!("pdfimages -png failed: {}", stderr.trim()),
            status_code: extract_output.status.code().map(|code| code as u16),
        });
    }

    let files = list_supported_image_files(&extract_dir)?;
    let mut warnings = Vec::new();
    if files.len() < entries.len() {
        warnings.push(format!(
            "pdfimages listed {} image(s) but extracted {} supported PNG/JPEG file(s)",
            entries.len(),
            files.len()
        ));
    }
    let aligned_entries = if files.len() == entries.len() {
        entries
    } else {
        dedupe_entries_by_object(entries)
    };

    let mut images_by_hash: BTreeMap<String, PdfImage> = BTreeMap::new();
    let mut skipped_filter = 0usize;
    for (entry, file) in aligned_entries.iter().zip(files.iter()) {
        let bytes = fs::read(file)?;
        if !image_survives_filter(entry.width, entry.height, bytes.len() as u64, pdf_policy) {
            skipped_filter += 1;
            continue;
        }
        let hash = sha256_hex(&bytes);
        if let Some(existing) = images_by_hash.get_mut(&hash) {
            for page in &entry.source_pages {
                if !existing.source_pages.contains(page) {
                    existing.source_pages.push(*page);
                }
            }
            continue;
        }
        images_by_hash.insert(
            hash,
            PdfImage {
                bytes,
                mime: mime_from_path(file).unwrap_or("image/png"),
                source_page: entry.source_page,
                source_pages: entry.source_pages.clone(),
                width: entry.width,
                height: entry.height,
                origin: PdfImageOrigin::Embedded {
                    xobject_name: entry.xobject_name.clone(),
                },
            },
        );
    }
    let _ = fs::remove_dir_all(&extract_dir);
    Ok(EmbeddedImageExtraction {
        images: images_by_hash.into_values().collect(),
        skipped_filter,
        warnings,
    })
}

async fn render_pdf_pages(path: &Path) -> Result<Vec<PdfImage>, DocsError> {
    let render_dir =
        std::env::temp_dir().join(format!("archon-pdf-render-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&render_dir)?;
    let prefix = render_dir.join("page");
    let output = Command::new(command_path("pdftoppm", "ARCHON_PDFTOPPM_BIN"))
        .arg("-png")
        .arg(path)
        .arg(&prefix)
        .output()
        .await
        .map_err(|e| DocsError::OcrApi {
            message: format!("pdftoppm not found or failed to start for PDF render. ({e})"),
            status_code: None,
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = fs::remove_dir_all(&render_dir);
        return Err(DocsError::OcrApi {
            message: format!("pdftoppm PDF render failed: {}", stderr.trim()),
            status_code: output.status.code().map(|code| code as u16),
        });
    }
    let files = list_supported_image_files(&render_dir)?;
    let mut images = Vec::with_capacity(files.len());
    for (index, file) in files.iter().enumerate() {
        let bytes = fs::read(file)?;
        let (width, height) = image_dimensions(&bytes).unwrap_or((0, 0));
        let page = index as u32 + 1;
        images.push(PdfImage {
            bytes,
            mime: mime_from_path(file).unwrap_or("image/png"),
            source_page: page,
            source_pages: vec![page],
            width,
            height,
            origin: PdfImageOrigin::RenderedPage,
        });
    }
    let _ = fs::remove_dir_all(&render_dir);
    Ok(images)
}

#[path = "pdf_images_list.rs"]
mod images_list;
// `pdf_tests` is `cfg(all(test, unix))`, so the re-export has to match it
// rather than plain `test`, or it is unused on Windows. See #136.
#[cfg(all(test, unix))]
pub(crate) use images_list::parse_pdfimages_size;
pub(crate) use images_list::{
    dedupe_entries_by_object, image_dimensions, list_supported_image_files, mime_from_path,
};
pub use images_list::{image_survives_filter, parse_pdfimages_list};

fn compute_pdf_page_offsets(text: &str) -> Vec<PageOffset> {
    let mut offsets = Vec::new();
    let mut page = 1u32;
    let mut char_start = 0usize;
    for (i, ch) in text.char_indices() {
        if ch == '\x0C' {
            offsets.push(PageOffset {
                page,
                char_start,
                char_end: i,
            });
            page += 1;
            char_start = i + 1;
        }
    }
    let text_len = text.len();
    if char_start < text_len || offsets.is_empty() {
        offsets.push(PageOffset {
            page,
            char_start,
            char_end: text_len,
        });
    }
    offsets
}

fn empty_page_offsets(page_count: u32) -> Vec<PageOffset> {
    (1..=page_count)
        .map(|page| PageOffset {
            page,
            char_start: 0,
            char_end: 0,
        })
        .collect()
}

// Unix-gated: every test in this module drives poppler through `#!/bin/sh`
// mocks made executable with `chmod 0755`. `PermissionsExt` does not exist on
// Windows and a shebang script is not executable there, so the module cannot
// compile at all on that target.
#[cfg(all(test, unix))]
#[path = "pdf_tests.rs"]
mod pdf_tests;
