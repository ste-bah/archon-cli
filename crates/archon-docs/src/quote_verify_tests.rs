use super::*;
use crate::hash::sha256_str;
use crate::models::{ChunkArtifact, ChunkSpatial};
use crate::schema::ensure_doc_schema;

// ---- pure helpers ---------------------------------------------------------------------------

#[test]
fn normalize_collapses_whitespace_and_folds_punct() {
    let n = normalize("  The   “quick”\n\tbrown—fox’s  ");
    let s: String = n.chars.iter().collect();
    assert_eq!(s, "the \"quick\" brown-fox's");
    // orig_byte has one entry per normalized char + a sentinel.
    assert_eq!(n.orig_byte.len(), n.chars.len() + 1);
}

#[test]
fn normalize_offset_map_recovers_verbatim_span() {
    // A match on the normalized text must map back to the VERBATIM original (smart quotes intact).
    let original = "He said “Being and Time” clearly.";
    let n = normalize(original);
    let q = normalize("being and time").chars;
    let start = find_subslice(&n.chars, &q).expect("found");
    let a = n.orig_byte[start];
    let b = n.orig_byte[start + q.len()];
    assert_eq!(&original[a..b], "Being and Time"); // original casing + no smart-quote bleed
}

#[test]
fn find_subslice_basic() {
    let hay: Vec<char> = "abcdef".chars().collect();
    assert_eq!(
        find_subslice(&hay, &"cde".chars().collect::<Vec<_>>()),
        Some(2)
    );
    assert_eq!(
        find_subslice(&hay, &"xyz".chars().collect::<Vec<_>>()),
        None
    );
    assert_eq!(find_subslice(&hay, &[]), None);
}

#[test]
fn approx_substring_similarity_scores() {
    let pat: Vec<char> = "energeia".chars().collect();
    // Exact substring inside a larger text → 1.0.
    let exact: Vec<char> = "the energeia of a body".chars().collect();
    assert!((approx_substring_similarity(&pat, &exact) - 1.0).abs() < 1e-9);
    // One substitution (energela) → 1 - 1/8.
    let one_edit: Vec<char> = "the energela of".chars().collect();
    assert!((approx_substring_similarity(&pat, &one_edit) - 0.875).abs() < 1e-9);
    // Unrelated → low.
    let none: Vec<char> = "completely different words".chars().collect();
    assert!(approx_substring_similarity(&pat, &none) < 0.5);
}

#[test]
fn reconstructed_attributes_range_to_crossed_chunks() {
    let chunks = vec![
        chunk("d", 0, 1, "first part of the"),
        chunk("d", 1, 1, "quote continues here"),
    ];
    let doc = Reconstructed::build(&chunks);
    // "the\nquote" spans the chunk boundary.
    let q = normalize("the quote").chars;
    let start = find_subslice(&doc.norm.chars, &q).expect("cross-chunk match");
    let a = doc.norm.orig_byte[start];
    let b = doc.norm.orig_byte[start + q.len()];
    assert_eq!(
        doc.chunks_in_range(a, b),
        vec![0, 1],
        "match crosses both chunks"
    );
}

// ---- DB-backed find_fragment_bboxes ---------------------------------------------------------

fn test_db() -> DbInstance {
    let db = DbInstance::new("mem", "", Default::default()).unwrap();
    ensure_doc_schema(&db).unwrap();
    db
}

fn chunk(doc: &str, idx: u32, page: u32, content: &str) -> ChunkArtifact {
    ChunkArtifact {
        chunk_id: format!("chunk-{doc}-{idx}"),
        document_id: doc.to_string(),
        artifact_id: format!("ocr-{doc}"),
        chunk_index: idx,
        page_start: page,
        page_end: page,
        content: content.to_string(),
        content_hash: sha256_str(content),
        embedding_status: "pending".into(),
    }
}

fn insert(db: &DbInstance, c: &ChunkArtifact, bbox: Option<&str>) {
    store::insert_chunk(db, c).unwrap();
    if let Some(b) = bbox {
        store::insert_chunk_spatial(
            db,
            &ChunkSpatial {
                chunk_id: c.chunk_id.clone(),
                page_num: c.page_start,
                super_box: b.to_string(),
                blocks: "[]".to_string(),
                coord_space: "marker".to_string(),
                spatial_hash: "h".to_string(),
            },
        )
        .unwrap();
    }
}

#[test]
fn exact_cross_chunk_match_returns_both_fragments_with_bboxes() {
    let db = test_db();
    insert(
        &db,
        &chunk("d", 0, 4, "the actualization of a living"),
        Some("[10,20,300,40]"),
    );
    insert(
        &db,
        &chunk("d", 1, 5, "body is its energeia and end"),
        Some("[10,50,300,70]"),
    );

    // Quote crosses the chunk boundary (page 4 → 5), with collapsed odd spacing.
    let loc = find_fragment_bboxes(&db, "d", "a living  body is its energeia and")
        .unwrap()
        .expect("located");
    assert_eq!(loc.match_kind, MatchKind::Exact);
    assert!((loc.similarity - 1.0).abs() < 1e-9);
    assert_eq!(loc.page_start, 4);
    assert_eq!(loc.page_end, 5);
    assert_eq!(loc.fragments.len(), 2, "spans both chunks");
    assert_eq!(loc.fragments[0].bbox, Some([10.0, 20.0, 300.0, 40.0]));
    assert_eq!(loc.fragments[1].page, 5);
    assert!(loc.source_span.contains("energeia"));
}

#[test]
fn fuzzy_match_reports_similarity_below_one() {
    let db = test_db();
    // Source says "phantasia"; the quote drifts to "phantasa" (one deletion) + wording change.
    insert(
        &db,
        &chunk("d", 0, 7, "imagination or phantasia is a movement"),
        Some("[0,0,100,20]"),
    );
    let loc = find_fragment_bboxes(&db, "d", "phantasa is a motion")
        .unwrap()
        .expect("fuzzy located");
    assert_eq!(loc.match_kind, MatchKind::Fuzzy);
    assert!(loc.similarity < 1.0 && loc.similarity >= REPORT_FLOOR);
}

#[test]
fn absent_quote_returns_none() {
    let db = test_db();
    insert(
        &db,
        &chunk("d", 0, 1, "the ecological approach to visual perception"),
        None,
    );
    assert!(
        find_fragment_bboxes(&db, "d", "quantum chromodynamics lagrangian")
            .unwrap()
            .is_none()
    );
}

#[test]
fn missing_bbox_yields_fragment_without_box() {
    let db = test_db();
    insert(&db, &chunk("d", 0, 3, "a chunk with no spatial row"), None); // pdftotext path
    let loc = find_fragment_bboxes(&db, "d", "chunk with no spatial")
        .unwrap()
        .expect("located");
    assert_eq!(loc.fragments.len(), 1);
    assert_eq!(loc.fragments[0].bbox, None);
    assert_eq!(loc.fragments[0].coord_space, "none");
}
