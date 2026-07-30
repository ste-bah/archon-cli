use super::*;
use lopdf::{Dictionary, Document, Object};

fn entry(page: u32, w: u32, h: u32, ppi: Option<u32>) -> PdfImagesListEntry {
    // Default to a large byte size so coverage tests aren't affected by the byte filter.
    entry_bytes(page, w, h, ppi, Some(500_000))
}

fn entry_bytes(
    page: u32,
    w: u32,
    h: u32,
    ppi: Option<u32>,
    bytes: Option<u64>,
) -> PdfImagesListEntry {
    PdfImagesListEntry {
        source_page: page,
        source_pages: vec![page],
        width: w,
        height: h,
        object_key: None,
        xobject_name: None,
        x_ppi: ppi,
        y_ppi: ppi,
        bytes,
    }
}

fn dims(pairs: &[(u32, f64, f64)]) -> BTreeMap<u32, (f64, f64)> {
    pairs.iter().map(|&(p, w, h)| (p, (w, h))).collect()
}

// ---- retained_images gate (matches the pipeline's embedded-image filter) ------------------

#[test]
fn retained_images_drops_page_shaped_but_tiny_bytes() {
    // The reviewer scenario: a large page-shaped image compressed to 40 bytes (JBIG2/CCITT).
    // The pipeline filters it by min_image_bytes; so must the classifier, or the report would
    // claim SCANNED BOOK while the pipeline enriches the survivors.
    let policy = PdfPolicy::default(); // min_image_dimension 200, min_image_bytes 4096
    let kept = retained_images(vec![entry_bytes(1, 1500, 2400, Some(0), Some(40))], &policy);
    assert!(
        kept.is_empty(),
        "40-byte image must be filtered like the pipeline"
    );
}

#[test]
fn retained_images_drops_small_dimension() {
    let policy = PdfPolicy::default();
    let kept = retained_images(
        vec![entry_bytes(1, 120, 90, Some(150), Some(50_000))],
        &policy,
    );
    assert!(kept.is_empty(), "max side 120 < 200 → filtered");
}

#[test]
fn retained_images_keeps_real_figures_and_unparsable_size() {
    let policy = PdfPolicy::default();
    let kept = retained_images(
        vec![
            entry_bytes(1, 1200, 800, Some(150), Some(80_000)), // real figure
            entry_bytes(2, 1200, 800, Some(150), None),         // size unknown → lenient keep
        ],
        &policy,
    );
    assert_eq!(kept.len(), 2);
}

#[test]
fn retained_images_honors_extract_embedded_images_false() {
    let mut policy = PdfPolicy::default();
    policy.extract_embedded_images = false;
    let kept = retained_images(vec![entry(1, 1500, 2400, Some(150))], &policy);
    assert!(
        kept.is_empty(),
        "pipeline extracts nothing → classifier counts nothing"
    );
}

// ---- ScanDetector::parse ------------------------------------------------------------------

#[test]
fn detector_parse_maps_known_and_defaults_to_union() {
    assert_eq!(ScanDetector::parse("aspect"), ScanDetector::Aspect);
    assert_eq!(ScanDetector::parse("coverage"), ScanDetector::Coverage);
    assert_eq!(ScanDetector::parse("COVERAGE"), ScanDetector::Coverage);
    assert_eq!(ScanDetector::parse("union"), ScanDetector::Union);
    assert_eq!(ScanDetector::parse("UNION"), ScanDetector::Union);
    // Unrecognized / empty → the default (union), not a weaker detector.
    assert_eq!(ScanDetector::parse("typo"), ScanDetector::Union);
    assert_eq!(ScanDetector::parse(""), ScanDetector::Union);
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
    // union verdict distinct from both bases (Konstan-shape: aspect catches, coverage misses).
    let uni = Some(UnionVerdict {
        scanned: true,
        page_scans: 9,
        low_confidence: false,
    });
    // aspect=true, coverage=false → divergent; aspect mode keeps aspect verdict.
    let (scanned, scans, divergent) = resolve(true, 7, &cov, &uni, ScanDetector::Aspect);
    assert!(scanned && scans == 7 && divergent);
    // coverage mode uses the coverage verdict.
    let (scanned, scans, divergent) = resolve(true, 7, &cov, &uni, ScanDetector::Coverage);
    assert!(!scanned && scans == 0 && divergent);
    // union mode uses the union verdict.
    let (scanned, scans, _divergent) = resolve(true, 7, &cov, &uni, ScanDetector::Union);
    assert!(scanned && scans == 9);
}

#[test]
fn resolve_coverage_unavailable_falls_back_to_aspect() {
    let (scanned, scans, divergent) = resolve(true, 5, &None, &None, ScanDetector::Coverage);
    assert!(
        scanned && scans == 5 && !divergent,
        "no coverage → aspect fallback, no divergence"
    );
    // Union with no page dims also falls back to aspect.
    let (scanned, scans, _) = resolve(true, 5, &None, &None, ScanDetector::Union);
    assert!(scanned && scans == 5);
}

// ---- union classifier ---------------------------------------------------------------------

#[test]
fn union_catches_margin_cropped_scan_aspect_only() {
    // Konstan-shape: page-scale image per page (aspect scan) but coverage ~0.73 (< 0.80).
    // A 1352x2134px @ 300ppi image on a 375x610pt page → 0.726 coverage; aspect page-scale.
    let page_dims = dims(&(1..=10).map(|p| (p, 375.0, 610.0)).collect::<Vec<_>>());
    let entries: Vec<_> = (1..=10).map(|p| entry(p, 1352, 2134, Some(300))).collect();
    let cov = classify_by_coverage(&entries, &page_dims, 10);
    assert!(!cov.scanned, "coverage alone misses the cropped scan");
    let uni = classify_by_union(&entries, &page_dims, 10);
    assert!(
        uni.scanned,
        "union catches it via the aspect page-scale test"
    );
    assert_eq!(uni.page_scans, 10);
}

#[test]
fn union_catches_low_dpi_scan_coverage_only() {
    // Low-DPI scan: 700x1140px @ 96ppi fills a 525x855pt page (~1.0 coverage) but min side 700
    // < 1000 → aspect misses it; coverage catches it; union agrees.
    let page_dims = dims(&(1..=10).map(|p| (p, 525.0, 855.0)).collect::<Vec<_>>());
    let entries: Vec<_> = (1..=10).map(|p| entry(p, 700, 1140, Some(96))).collect();
    assert!(
        entries.iter().all(|e| e.width.min(e.height) < 1000),
        "sub-1000px so aspect's floor misses it"
    );
    let uni = classify_by_union(&entries, &page_dims, 10);
    assert!(uni.scanned);
}

#[test]
fn union_leaves_born_digital_alone() {
    // Small clustered figures: neither aspect nor coverage flags a scan → union born-digital.
    let page_dims = dims(&(1..=10).map(|p| (p, 612.0, 792.0)).collect::<Vec<_>>());
    let entries: Vec<_> = [2u32, 5, 9]
        .iter()
        .map(|&p| entry(p, 300, 300, Some(150)))
        .collect();
    let uni = classify_by_union(&entries, &page_dims, 10);
    assert!(!uni.scanned);
    assert_eq!(uni.page_scans, 0);
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
