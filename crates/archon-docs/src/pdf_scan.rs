//! Adoption #2 — true page-dimension **coverage** classifier, run A/B against the shipped
//! **aspect** heuristic ([`crate::pdf_image_enrichment::is_scanned_page_images`]).
//!
//! The aspect heuristic detects a full-page scan by pixel dims alone (large AND page-shaped),
//! which couples two guesses: a DPI-dependent size floor and a fixed aspect range. Coverage
//! removes both by asking how much of the page an image actually occupies:
//!
//! ```text
//! drawn_width_pt  = pixel_width  * 72 / x_ppi      // x_ppi from `pdfimages -list`
//! drawn_height_pt = pixel_height * 72 / y_ppi
//! coverage        = (drawn_width_pt / page_width_pt) * (drawn_height_pt / page_height_pt)
//! ```
//!
//! The only new input is per-page dimensions (the `MediaBox`, in points), read with `lopdf`. Both
//! detectors run on every classified PDF; the policy knob `scan_detector` selects which verdict is
//! ACTIVE, and any disagreement is logged loudly — that divergence log is the A/B signal that tells
//! us (on the real corpus) whether coverage is ready to become the default.

use std::collections::BTreeMap;
use std::path::Path;

use archon_policy::PdfPolicy;

use crate::pdf::{PdfImage, PdfImageOrigin, PdfImagesListEntry};
use crate::pdf_image_enrichment::{is_page_scale, is_scanned_page_images};

/// A page whose (capped) total image coverage is at least this fraction is a full-page scan.
const PAGE_SCAN_COVERAGE: f64 = 0.80;
/// ppi below this is unusable (JBIG2/CCITT report 0/1) — defer such images to the aspect test.
const MIN_USABLE_PPI: u32 = 10;
/// Fraction of pages that must be full-page scans for the doc to be a "scanned book". Matches the
/// aspect heuristic's threshold so the two detectors are compared on equal footing.
const SCANNED_BOOK_FRACTION: f64 = 0.70;
/// Union third arm: a page whose ONLY image covers at least half the page is a margin-cropped
/// scan. The Kassel Rhetorica measured 275/279 pages as exactly-one-image at 0.5–0.8 coverage
/// (text block scanned without margins) — under both the 0.80 coverage bar and the aspect pixel
/// floor, so the union read 61% < 70% and called a scanned book BORN-DIGITAL. Requiring the
/// singleton keeps born-digital immunity: papers hit occasional full-page figures, but not one
/// per page across ≥70% of the document.
const CROPPED_SCAN_COVERAGE: f64 = 0.50;

/// Which scan detector governs the active enrichment decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanDetector {
    /// Shipped pixel-dims heuristic (large AND page-shaped) alone. Misses low-DPI scans.
    Aspect,
    /// True page-coverage % via ppi + `MediaBox` alone. Misses margin-cropped scans.
    Coverage,
    /// A page is a scan if aspect OR coverage flags it — catches both low-DPI scans (coverage) and
    /// margin-cropped scans (aspect). Corpus-validated as strictly better; the SHIPPED DEFAULT.
    Union,
}

impl ScanDetector {
    /// Parse the policy string. `"aspect"`/`"coverage"` select a single base detector; anything
    /// else (including the default and any unrecognized value) is `Union`, so a typo degrades to
    /// the best detector rather than a weaker one. (The policy loader already rejects unknown values
    /// and keeps the default, so `parse` sees only validated strings in the normal flow.)
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "aspect" => ScanDetector::Aspect,
            "coverage" => ScanDetector::Coverage,
            _ => ScanDetector::Union,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ScanDetector::Aspect => "aspect",
            ScanDetector::Coverage => "coverage",
            ScanDetector::Union => "union",
        }
    }
}

/// The union classifier's verdict for a whole document.
#[derive(Debug, Clone, PartialEq)]
pub struct UnionVerdict {
    /// ≥70% of pages are a scan by aspect OR coverage.
    pub scanned: bool,
    /// Pages flagged a scan by either base detector.
    pub page_scans: usize,
    /// ≥1 image was deferred inside the coverage half (unusable ppi / missing dims).
    pub low_confidence: bool,
}

/// The coverage classifier's verdict for a whole document.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverageVerdict {
    /// Most pages are full-page scans → treat as a scanned book (skip enrichment).
    pub scanned: bool,
    /// Number of pages whose capped coverage reached [`PAGE_SCAN_COVERAGE`].
    pub page_scans: usize,
    /// Peak per-page capped coverage across the doc (diagnostic for the report).
    pub max_coverage: f64,
    /// ≥1 image could not be measured by coverage (unusable ppi or missing page dims) and was
    /// deferred to the aspect page-scale test; the verdict is advisory — flag the doc for review.
    pub low_confidence: bool,
    /// Count of deferred images (drives [`Self::low_confidence`]).
    pub deferred_images: usize,
}

/// The full A/B classification: both detector verdicts + the active selection.
#[derive(Debug, Clone)]
pub struct ScanClassification {
    pub page_count: u32,
    pub embedded_images: usize,
    /// Detector that produced the active verdict.
    pub detector: ScanDetector,
    pub active_scanned: bool,
    pub active_page_scans: usize,
    pub aspect_scanned: bool,
    pub aspect_page_scans: usize,
    /// `None` when page dimensions could not be read at all (no `lopdf` parse / no `MediaBox`).
    pub coverage: Option<CoverageVerdict>,
    /// The two detectors disagree on the scanned/born-digital verdict.
    pub divergent: bool,
}

/// Classify a PDF with BOTH detectors and select the active verdict per `detector`. Reads only
/// `pdfimages -list` (dims + ppi + size), `pdfinfo` (page count), and `lopdf` (page dims) — no byte
/// extraction, no Marker. Applies the SAME image gate the ingest pipeline applies
/// (`extract_embedded_images` + `min_image_dimension`/`min_image_bytes`), so the pre-ingest verdict
/// tracks what enrichment will actually do rather than counting images the pipeline will discard.
/// Logs loudly when the two detectors disagree.
pub fn classify_scan(
    path: &Path,
    detector: ScanDetector,
    pdf_policy: &PdfPolicy,
) -> ScanClassification {
    let entries = retained_images(crate::pdf::list_embedded_image_dims(path), pdf_policy);
    let page_count = crate::pdf::pdf_page_count(path).unwrap_or(0);

    // Aspect: run the shipped heuristic on images synthesized from the list (dims only).
    let images: Vec<PdfImage> = entries.iter().map(synth_image).collect();
    let aspect_scanned = is_scanned_page_images(&images, page_count);
    let aspect_page_scans = images.iter().filter(|i| is_page_scale(i)).count();

    // Coverage + union: both need per-page dims. If none are readable, both are unavailable (None),
    // NOT a silent mirror of aspect.
    let page_dims = page_dimensions(path);
    let (coverage, union) = if page_dims.is_empty() {
        (None, None)
    } else {
        (
            Some(classify_by_coverage(&entries, &page_dims, page_count)),
            Some(classify_by_union(&entries, &page_dims, page_count)),
        )
    };

    let (active_scanned, active_page_scans, divergent) = resolve(
        aspect_scanned,
        aspect_page_scans,
        &coverage,
        &union,
        detector,
    );

    if divergent {
        let cov = coverage.as_ref();
        tracing::warn!(
            doc = %path.display(),
            aspect_scanned,
            coverage_scanned = cov.map(|c| c.scanned),
            coverage_max = cov.map(|c| c.max_coverage),
            low_confidence = cov.map(|c| c.low_confidence).unwrap_or(false),
            "scan-detector DISAGREEMENT (aspect vs coverage) — union took the OR; review these docs"
        );
    }

    ScanClassification {
        page_count,
        embedded_images: entries.len(),
        detector,
        active_scanned,
        active_page_scans,
        aspect_scanned,
        aspect_page_scans,
        coverage,
        divergent,
    }
}

/// Select the active verdict and compute divergence. Coverage governs only when it is available;
/// otherwise we fall back to aspect even under `ScanDetector::Coverage` (can't decide by coverage
/// with no page dims). Split out from [`classify_scan`] so the selection logic is unit-testable
/// without touching the filesystem.
fn resolve(
    aspect_scanned: bool,
    aspect_page_scans: usize,
    coverage: &Option<CoverageVerdict>,
    union: &Option<UnionVerdict>,
    detector: ScanDetector,
) -> (bool, usize, bool) {
    let divergent = coverage
        .as_ref()
        .is_some_and(|c| c.scanned != aspect_scanned);
    // Coverage/Union govern only when available; otherwise fall back to aspect (can't decide by
    // page coverage with no page dims).
    let (active_scanned, active_page_scans) = match detector {
        ScanDetector::Aspect => (aspect_scanned, aspect_page_scans),
        ScanDetector::Coverage => coverage
            .as_ref()
            .map(|c| (c.scanned, c.page_scans))
            .unwrap_or((aspect_scanned, aspect_page_scans)),
        ScanDetector::Union => union
            .as_ref()
            .map(|u| (u.scanned, u.page_scans))
            .unwrap_or((aspect_scanned, aspect_page_scans)),
    };
    (active_scanned, active_page_scans, divergent)
}

/// Apply the pipeline's embedded-image gate to `pdfimages -list` entries so the pre-ingest verdict
/// counts the same images enrichment will: honor `extract_embedded_images` (false → the pipeline
/// extracts nothing), then filter by `min_image_dimension` (max side) and `min_image_bytes`,
/// measured the way the PIPELINE measures it: against the extracted-file size, which for a
/// bi-level codec is bounded below by the decoded bitmap (`w·h/8`), never the compressed stream.
/// JBIG2/CCITT compress a full page scan to ~1–3 KB of stream — the White 1985 scan's 23 page
/// images all sat under the 4096-byte gate by stream size, so the classifier saw "no embedded
/// images" while the pipeline extracted and enriched all 23 (the two stages disagreed about
/// which images exist). `max(stream, w·h/8)` keeps genuinely tiny decorative images filtered
/// while making page-scale bi-level scans visible to scan detection. An entry with an
/// unparsable size passes the byte gate (lenient: never drop an image by guessing it away).
///
/// NOTE: unlike the pipeline this does NOT dedupe by object/hash — `pdfimages` already lists a shared
/// XObject once per page it is drawn on, which is exactly the per-page granularity the coverage
/// classifier needs; deduping would collapse those rows and break the per-page coverage sum. (The
/// pipeline's aspect path dedupes then counts by a single `source_page`, which under-counts shared
/// images — a pre-existing quirk, not introduced here.)
fn retained_images(
    entries: Vec<PdfImagesListEntry>,
    pdf_policy: &PdfPolicy,
) -> Vec<PdfImagesListEntry> {
    if !pdf_policy.extract_embedded_images {
        return Vec::new();
    }
    entries
        .into_iter()
        .filter(|e| {
            let decoded_floor = (e.width as u64 * e.height as u64) / 8;
            e.width.max(e.height) >= pdf_policy.min_image_dimension
                && e.bytes
                    .is_none_or(|b| b.max(decoded_floor) >= pdf_policy.min_image_bytes)
        })
        .collect()
}

/// Coverage classifier: per-image `coverage = (px·72/ppi)/page_pt`, summed per page and capped at
/// 1.0 (overlapping thumbnail+full-res pairs otherwise exceed 1.0). An image with unusable ppi or
/// no page dims is **deferred to the aspect page-scale test** (pixel-only) for its contribution —
/// never assumed to fill the page (that would silently eat born-digital line-art figures, which are
/// often CCITT with ppi 0/1) — and the doc is marked low-confidence.
pub fn classify_by_coverage(
    entries: &[PdfImagesListEntry],
    page_dims: &BTreeMap<u32, (f64, f64)>,
    page_count: u32,
) -> CoverageVerdict {
    let (per_page, deferred) = coverage_per_page(entries, page_dims);
    let mut page_scans = 0usize;
    let mut max_coverage = 0.0f64;
    for cov in per_page.values() {
        max_coverage = max_coverage.max(*cov);
        if *cov >= PAGE_SCAN_COVERAGE {
            page_scans += 1;
        }
    }
    let scanned = page_count > 0 && page_scans as f64 / page_count as f64 >= SCANNED_BOOK_FRACTION;
    CoverageVerdict {
        scanned,
        page_scans,
        max_coverage,
        low_confidence: deferred > 0,
        deferred_images: deferred,
    }
}

/// Per-page (capped) coverage map + count of images deferred to the aspect test (unusable ppi /
/// missing dims). Shared by the coverage and union classifiers so they compute coverage identically.
fn coverage_per_page(
    entries: &[PdfImagesListEntry],
    page_dims: &BTreeMap<u32, (f64, f64)>,
) -> (BTreeMap<u32, f64>, usize) {
    let mut per_page: BTreeMap<u32, f64> = BTreeMap::new();
    let mut deferred = 0usize;
    for e in entries {
        let contribution = match coverage_of(e, page_dims) {
            Some(cov) => cov,
            None => {
                deferred += 1;
                aspect_contribution(e)
            }
        };
        *per_page.entry(e.source_page).or_insert(0.0) += contribution;
    }
    for cov in per_page.values_mut() {
        *cov = cov.min(1.0);
    }
    (per_page, deferred)
}

/// Union classifier: a page is a scan if it is page-scale by the aspect test OR its coverage reaches
/// [`PAGE_SCAN_COVERAGE`]. Corpus dry-run showed neither base detector alone is complete — aspect
/// misses low-DPI scans (below its 1000px pixel floor) while coverage misses margin-cropped scans
/// (text-block-only images that fill ~0.73 of the page). Their union classifies every divergent
/// corpus doc correctly.
pub fn classify_by_union(
    entries: &[PdfImagesListEntry],
    page_dims: &BTreeMap<u32, (f64, f64)>,
    page_count: u32,
) -> UnionVerdict {
    let (per_page_cov, deferred) = coverage_per_page(entries, page_dims);
    let mut scan_pages = aspect_scan_page_set(entries);
    let mut per_page_count: BTreeMap<u32, usize> = BTreeMap::new();
    for e in entries {
        *per_page_count.entry(e.source_page).or_insert(0) += 1;
    }
    for (page, cov) in &per_page_cov {
        if *cov >= PAGE_SCAN_COVERAGE {
            scan_pages.insert(*page);
        }
        // Third arm: margin-cropped scan — the page's ONLY image covering ≥ half the page.
        if *cov >= CROPPED_SCAN_COVERAGE && per_page_count.get(page) == Some(&1) {
            scan_pages.insert(*page);
        }
    }
    let page_scans = scan_pages.len();
    let scanned = page_count > 0 && page_scans as f64 / page_count as f64 >= SCANNED_BOOK_FRACTION;
    UnionVerdict {
        scanned,
        page_scans,
        low_confidence: deferred > 0,
    }
}

/// Pages the aspect heuristic counts as scans — exactly one embedded image and it is page-scale.
/// Mirrors the per-page rule inside [`is_scanned_page_images`], exposed for the union detector.
fn aspect_scan_page_set(entries: &[PdfImagesListEntry]) -> std::collections::BTreeSet<u32> {
    let mut per_page: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    for e in entries {
        let counts = per_page.entry(e.source_page).or_insert((0, 0));
        counts.1 += 1;
        if is_page_scale(&synth_image(e)) {
            counts.0 += 1;
        }
    }
    per_page
        .into_iter()
        .filter(|(_, (page_scale, embedded))| *page_scale == 1 && *embedded == 1)
        .map(|(page, _)| page)
        .collect()
}

/// The measurable coverage of one image, or `None` when it must defer to the aspect test (no ppi
/// column, unusable ppi, or no page dims for its page).
fn coverage_of(e: &PdfImagesListEntry, page_dims: &BTreeMap<u32, (f64, f64)>) -> Option<f64> {
    let (x_ppi, y_ppi) = (e.x_ppi?, e.y_ppi?);
    if x_ppi < MIN_USABLE_PPI || y_ppi < MIN_USABLE_PPI {
        return None;
    }
    let &(page_w, page_h) = page_dims.get(&e.source_page)?;
    if page_w <= 0.0 || page_h <= 0.0 {
        return None;
    }
    let drawn_w = e.width as f64 * 72.0 / x_ppi as f64;
    let drawn_h = e.height as f64 * 72.0 / y_ppi as f64;
    Some((drawn_w / page_w) * (drawn_h / page_h))
}

/// Fallback contribution for a deferred image: 1.0 if it is page-scale by the aspect pixel test,
/// else 0.0. Reuses [`is_page_scale`] so the aspect page-scale definition stays single-sourced.
fn aspect_contribution(e: &PdfImagesListEntry) -> f64 {
    if is_page_scale(&synth_image(e)) {
        1.0
    } else {
        0.0
    }
}

fn synth_image(e: &PdfImagesListEntry) -> PdfImage {
    PdfImage {
        bytes: Vec::new(),
        mime: "",
        source_page: e.source_page,
        source_pages: e.source_pages.clone(),
        width: e.width,
        height: e.height,
        origin: PdfImageOrigin::Embedded {
            xobject_name: e.xobject_name.clone(),
        },
    }
}

/// Per-page dimensions (width_pt, height_pt) from the `MediaBox`, keyed by 1-based page number.
/// Empty when the PDF can't be parsed by `lopdf` — the caller treats that as "coverage
/// unavailable". `lopdf` loads the whole file eagerly (a transient cost, fine on these machines).
pub fn page_dimensions(path: &Path) -> BTreeMap<u32, (f64, f64)> {
    let mut out = BTreeMap::new();
    let Ok(doc) = lopdf::Document::load(path) else {
        return out;
    };
    for (number, page_id) in doc.get_pages() {
        if let Some(dims) = get_media_box(&doc, page_id) {
            out.insert(number, dims);
        }
    }
    out
}

/// Read a page's `MediaBox` (width_pt, height_pt), walking `Parent` because `MediaBox` is
/// inheritable — most scanners set it once on the page-tree root, so a naive per-page lookup returns
/// nothing on exactly the uniform scanned books we care about. Guards for a malformed corpus:
/// - **cycle guard** — cap the `Parent` climb at depth 10 (terminates firmware-bug `Parent` loops);
/// - **array guard** — require ≥4 numeric elements, so a truncated `[…]` degrades to `None`;
/// - **Integer *and* Real** — `MediaBox` values are usually integers (`[0 0 612 792]`), which
///   `lopdf`'s `as_f64` rejects; [`as_num`] accepts both so the common case doesn't fall through.
fn get_media_box(doc: &lopdf::Document, page_id: lopdf::ObjectId) -> Option<(f64, f64)> {
    let mut cur = page_id;
    for _ in 0..10 {
        let dict = doc.get_object(cur).ok()?.as_dict().ok()?;
        if let Ok(mb) = dict.get(b"MediaBox") {
            let arr = mb.as_array().ok()?;
            if arr.len() < 4 {
                return None;
            }
            let x0 = as_num(&arr[0])?;
            let y0 = as_num(&arr[1])?;
            let x1 = as_num(&arr[2])?;
            let y1 = as_num(&arr[3])?;
            let (w, h) = (x1 - x0, y1 - y0);
            return (w > 0.0 && h > 0.0).then_some((w, h));
        }
        cur = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

/// `MediaBox` numbers may be `Integer` or `Real`; `lopdf`'s `as_f64` only accepts `Real`.
fn as_num(o: &lopdf::Object) -> Option<f64> {
    match o {
        lopdf::Object::Integer(i) => Some(*i as f64),
        lopdf::Object::Real(r) => Some(*r as f64),
        _ => None,
    }
}

#[cfg(test)]
#[path = "pdf_scan_tests.rs"]
mod tests;
