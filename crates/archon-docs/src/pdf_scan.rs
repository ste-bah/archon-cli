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

use crate::pdf::{PdfImage, PdfImageOrigin, PdfImagesListEntry};
use crate::pdf_image_enrichment::{is_page_scale, is_scanned_page_images};

/// A page whose (capped) total image coverage is at least this fraction is a full-page scan.
const PAGE_SCAN_COVERAGE: f64 = 0.80;
/// ppi below this is unusable (JBIG2/CCITT report 0/1) — defer such images to the aspect test.
const MIN_USABLE_PPI: u32 = 10;
/// Fraction of pages that must be full-page scans for the doc to be a "scanned book". Matches the
/// aspect heuristic's threshold so the two detectors are compared on equal footing.
const SCANNED_BOOK_FRACTION: f64 = 0.70;

/// Which scan detector governs the active enrichment decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanDetector {
    /// Shipped pixel-dims heuristic (large AND page-shaped). The default until coverage is proven.
    Aspect,
    /// True page-coverage % via ppi + `MediaBox`.
    Coverage,
}

impl ScanDetector {
    /// Parse the policy string; anything other than an explicit `"coverage"` is `Aspect` (the safe
    /// default), so a typo can never silently flip the corpus onto the unproven detector.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "coverage" => ScanDetector::Coverage,
            _ => ScanDetector::Aspect,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ScanDetector::Aspect => "aspect",
            ScanDetector::Coverage => "coverage",
        }
    }
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
/// `pdfimages -list` (dims + ppi), `pdfinfo` (page count), and `lopdf` (page dims) — no byte
/// extraction, no Marker. Logs loudly when the two detectors disagree.
pub fn classify_scan(path: &Path, detector: ScanDetector) -> ScanClassification {
    let entries = crate::pdf::list_embedded_image_dims(path);
    let page_count = crate::pdf::pdf_page_count(path).unwrap_or(0);

    // Aspect: run the shipped heuristic on images synthesized from the list (dims only).
    let images: Vec<PdfImage> = entries.iter().map(synth_image).collect();
    let aspect_scanned = is_scanned_page_images(&images, page_count);
    let aspect_page_scans = images.iter().filter(|i| is_page_scale(i)).count();

    // Coverage: needs per-page dims. If none are readable, coverage is unavailable (None), NOT a
    // silent mirror of aspect.
    let page_dims = page_dimensions(path);
    let coverage = if page_dims.is_empty() {
        None
    } else {
        Some(classify_by_coverage(&entries, &page_dims, page_count))
    };

    let (active_scanned, active_page_scans, divergent) =
        resolve(aspect_scanned, aspect_page_scans, &coverage, detector);

    if divergent {
        let cov = coverage.as_ref();
        tracing::warn!(
            doc = %path.display(),
            aspect_scanned,
            coverage_scanned = cov.map(|c| c.scanned),
            coverage_max = cov.map(|c| c.max_coverage),
            low_confidence = cov.map(|c| c.low_confidence).unwrap_or(false),
            "scan-detector DISAGREEMENT (aspect vs coverage) — review before flipping the default"
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
    detector: ScanDetector,
) -> (bool, usize, bool) {
    let divergent = coverage
        .as_ref()
        .is_some_and(|c| c.scanned != aspect_scanned);
    let (active_scanned, active_page_scans) = match (detector, coverage) {
        (ScanDetector::Coverage, Some(c)) => (c.scanned, c.page_scans),
        // Coverage requested but unavailable → aspect fallback; or aspect selected outright.
        _ => (aspect_scanned, aspect_page_scans),
    };
    (active_scanned, active_page_scans, divergent)
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

    let mut page_scans = 0usize;
    let mut max_coverage = 0.0f64;
    for cov in per_page.values_mut() {
        *cov = cov.min(1.0);
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
mod tests {
    use super::*;
    use lopdf::{Dictionary, Document, Object};

    fn entry(page: u32, w: u32, h: u32, ppi: Option<u32>) -> PdfImagesListEntry {
        PdfImagesListEntry {
            source_page: page,
            source_pages: vec![page],
            width: w,
            height: h,
            object_key: None,
            xobject_name: None,
            x_ppi: ppi,
            y_ppi: ppi,
            bytes: Some(500_000),
        }
    }

    fn dims(pairs: &[(u32, f64, f64)]) -> BTreeMap<u32, (f64, f64)> {
        pairs.iter().map(|&(p, w, h)| (p, (w, h))).collect()
    }

    // ---- ScanDetector::parse ------------------------------------------------------------------

    #[test]
    fn detector_parse_defaults_to_aspect() {
        assert_eq!(ScanDetector::parse("coverage"), ScanDetector::Coverage);
        assert_eq!(ScanDetector::parse("COVERAGE"), ScanDetector::Coverage);
        assert_eq!(ScanDetector::parse("aspect"), ScanDetector::Aspect);
        assert_eq!(ScanDetector::parse("typo"), ScanDetector::Aspect);
        assert_eq!(ScanDetector::parse(""), ScanDetector::Aspect);
    }

    // ---- coverage classifier ------------------------------------------------------------------

    #[test]
    fn full_page_scans_are_detected_by_coverage() {
        // Uexküll: 1303 px @ 241 ppi on a 389 pt-wide page → ~100% width coverage, one per page.
        let page_dims = dims(&(1..=5).map(|p| (p, 389.0, 611.0)).collect::<Vec<_>>());
        let entries: Vec<_> = (1..=5).map(|p| entry(p, 1303, 2041, Some(241))).collect();
        let v = classify_by_coverage(&entries, &page_dims, 5);
        assert!(v.scanned, "5/5 full-page scans should be a scanned book");
        assert_eq!(v.page_scans, 5);
        assert!(v.max_coverage > 0.95 && v.max_coverage <= 1.0);
        assert!(!v.low_confidence);
    }

    #[test]
    fn small_figures_are_not_scans_by_coverage() {
        // A small 300 px @ 150 ppi figure (144 pt) on a 612 pt page → ~0.05 coverage.
        let page_dims = dims(&(1..=10).map(|p| (p, 612.0, 792.0)).collect::<Vec<_>>());
        let entries: Vec<_> = [2u32, 5, 9]
            .iter()
            .map(|&p| entry(p, 300, 300, Some(150)))
            .collect();
        let v = classify_by_coverage(&entries, &page_dims, 10);
        assert!(!v.scanned);
        assert_eq!(v.page_scans, 0);
        assert!(v.max_coverage < 0.2);
    }

    #[test]
    fn unusable_ppi_defers_to_aspect_and_flags_low_confidence() {
        // ppi = 0 (JBIG2/CCITT) → cannot compute coverage; defers to the aspect page-scale test.
        // A large page-shaped image still counts (aspect says page-scale); low-confidence is set.
        let page_dims = dims(&(1..=4).map(|p| (p, 612.0, 792.0)).collect::<Vec<_>>());
        let entries: Vec<_> = (1..=4).map(|p| entry(p, 1303, 2041, Some(0))).collect();
        let v = classify_by_coverage(&entries, &page_dims, 4);
        assert!(
            v.scanned,
            "deferred-but-page-scale images should still read as scans"
        );
        assert!(v.low_confidence);
        assert_eq!(v.deferred_images, 4);
    }

    #[test]
    fn unusable_ppi_small_lineart_is_not_a_scan() {
        // CCITT line-art figure (ppi 0) that is small/non-page-shaped must NOT be assumed full-page.
        let page_dims = dims(&(1..=6).map(|p| (p, 612.0, 792.0)).collect::<Vec<_>>());
        let entries: Vec<_> = [2u32, 4]
            .iter()
            .map(|&p| entry(p, 400, 300, Some(0)))
            .collect();
        let v = classify_by_coverage(&entries, &page_dims, 6);
        assert!(!v.scanned);
        assert_eq!(v.page_scans, 0);
        assert!(v.low_confidence, "deferral still flags low-confidence");
    }

    #[test]
    fn multi_strip_page_coverage_is_capped_at_one() {
        // A page scanned as two full-width half-height strips (each 0.5 coverage → sum 1.0) plus a
        // thumbnail of the same page (pushes the raw sum past 1.0) → capped to 1.0, one scan. This
        // is the concrete A/B win: the aspect "exactly one image per page" rule misses legit
        // multi-image scans. On a 612×792 page at 150 ppi:
        //   strip  1275×825 → drawn 612×396 pt → coverage (612/612)*(396/792) = 0.50
        //   thumb   300×400 → drawn 144×192 pt → coverage (144/612)*(192/792) ≈ 0.057
        let page_dims = dims(&[(1, 612.0, 792.0)]);
        let entries = vec![
            entry(1, 1275, 825, Some(150)), // 0.50 coverage
            entry(1, 1275, 825, Some(150)), // 0.50 coverage → sum 1.00
            entry(1, 300, 400, Some(150)),  // thumbnail → raw sum ≈ 1.057
        ];
        let v = classify_by_coverage(&entries, &page_dims, 1);
        assert_eq!(v.page_scans, 1);
        assert!((v.max_coverage - 1.0).abs() < 1e-9, "capped at 1.0");
    }

    #[test]
    fn missing_page_dims_defers_that_page() {
        // Page 3 has no dims → its image defers to aspect; other pages measured by coverage.
        let page_dims = dims(&[(1, 389.0, 611.0), (2, 389.0, 611.0)]);
        let entries = vec![
            entry(1, 1303, 2041, Some(241)),
            entry(2, 1303, 2041, Some(241)),
            entry(3, 1303, 2041, Some(241)), // no dims → deferred (still page-scale by aspect)
        ];
        let v = classify_by_coverage(&entries, &page_dims, 3);
        assert_eq!(v.deferred_images, 1);
        assert!(v.low_confidence);
        assert_eq!(v.page_scans, 3, "deferred page still page-scale by aspect");
    }

    // ---- resolve / selection ------------------------------------------------------------------

    #[test]
    fn resolve_selects_active_and_detects_divergence() {
        let cov = Some(CoverageVerdict {
            scanned: false,
            page_scans: 0,
            max_coverage: 0.1,
            low_confidence: false,
            deferred_images: 0,
        });
        // aspect=true, coverage=false → divergent; aspect mode keeps aspect verdict.
        let (scanned, scans, divergent) = resolve(true, 7, &cov, ScanDetector::Aspect);
        assert!(scanned && scans == 7 && divergent);
        // coverage mode uses the coverage verdict.
        let (scanned, scans, divergent) = resolve(true, 7, &cov, ScanDetector::Coverage);
        assert!(!scanned && scans == 0 && divergent);
    }

    #[test]
    fn resolve_coverage_unavailable_falls_back_to_aspect() {
        let (scanned, scans, divergent) = resolve(true, 5, &None, ScanDetector::Coverage);
        assert!(
            scanned && scans == 5 && !divergent,
            "no coverage → aspect fallback, no divergence"
        );
    }

    // ---- get_media_box (lopdf) ----------------------------------------------------------------

    fn page_with(mb: Option<Object>, parent: Option<lopdf::ObjectId>) -> Dictionary {
        let mut d = Dictionary::new();
        d.set("Type", Object::Name(b"Page".to_vec()));
        if let Some(mb) = mb {
            d.set("MediaBox", mb);
        }
        if let Some(p) = parent {
            d.set("Parent", Object::Reference(p));
        }
        d
    }

    fn int_box(x0: i64, y0: i64, x1: i64, y1: i64) -> Object {
        Object::Array(vec![
            Object::Integer(x0),
            Object::Integer(y0),
            Object::Integer(x1),
            Object::Integer(y1),
        ])
    }

    #[test]
    fn media_box_direct_integer() {
        let mut doc = Document::with_version("1.5");
        let id = doc.add_object(page_with(Some(int_box(0, 0, 612, 792)), None));
        assert_eq!(get_media_box(&doc, id), Some((612.0, 792.0)));
    }

    #[test]
    fn media_box_real_values() {
        let mut doc = Document::with_version("1.5");
        let mb = Object::Array(vec![
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(612.5),
            Object::Real(792.25),
        ]);
        let id = doc.add_object(page_with(Some(mb), None));
        assert_eq!(get_media_box(&doc, id), Some((612.5, 792.25)));
    }

    #[test]
    fn media_box_inherited_from_parent() {
        let mut doc = Document::with_version("1.5");
        let parent = doc.add_object(page_with(Some(int_box(0, 0, 595, 842)), None));
        let child = doc.add_object(page_with(None, Some(parent)));
        assert_eq!(get_media_box(&doc, child), Some((595.0, 842.0)));
    }

    #[test]
    fn media_box_absent_everywhere_is_none() {
        let mut doc = Document::with_version("1.5");
        let id = doc.add_object(page_with(None, None));
        assert_eq!(get_media_box(&doc, id), None);
    }

    #[test]
    fn media_box_parent_cycle_terminates() {
        // A.Parent = B, B.Parent = A, neither has a MediaBox → the depth cap terminates the climb.
        let mut doc = Document::with_version("1.5");
        let a_id = doc.add_object(page_with(None, None));
        let b_id = doc.add_object(page_with(None, Some(a_id)));
        // Point A back at B to form the cycle.
        if let Ok(a) = doc.get_object_mut(a_id).and_then(|o| o.as_dict_mut()) {
            a.set("Parent", Object::Reference(b_id));
        }
        assert_eq!(get_media_box(&doc, a_id), None);
    }

    #[test]
    fn media_box_truncated_array_is_none() {
        let mut doc = Document::with_version("1.5");
        let mb = Object::Array(vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(612),
        ]);
        let id = doc.add_object(page_with(Some(mb), None));
        assert_eq!(get_media_box(&doc, id), None);
    }

    #[test]
    fn media_box_zero_area_is_none() {
        let mut doc = Document::with_version("1.5");
        let id = doc.add_object(page_with(Some(int_box(0, 0, 0, 792)), None));
        assert_eq!(get_media_box(&doc, id), None);
    }
}
