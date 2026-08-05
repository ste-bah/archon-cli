//! End-to-end tests for the born-digital native-coordinate ingest path
//! (`pdf_native_source` + the ingest_pdf routing). NO mocks: these run the real
//! pdftotext/pdfimages toolchain, the real scan detector, and the real Python
//! sidecar against a synthesized-but-valid PDF.

use super::test_support::*;
use super::*;

/// Build a small but fully VALID born-digital PDF (correct xref offsets) so the real
/// toolchain — pdftotext, pdfimages, the scan detector, and the native sidecar — all run
/// against it with no mocks. Uncompressed Helvetica text, one content stream per page.
fn minimal_born_digital_pdf(pages: &[&[&str]]) -> Vec<u8> {
    fn content(lines: &[&str]) -> String {
        let mut s = String::new();
        let mut y = 700;
        for l in lines {
            s.push_str(&format!("BT /F1 12 Tf 72 {y} Td ({l}) Tj ET\n"));
            y -= 18;
        }
        s
    }
    let font_obj = 3 + pages.len() * 2; // catalog, pages, then (page, content) pairs
    let kids = (0..pages.len())
        .map(|i| format!("{} 0 R", 3 + i * 2))
        .collect::<Vec<_>>()
        .join(" ");
    let mut objs = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", pages.len()),
    ];
    for (i, lines) in pages.iter().enumerate() {
        objs.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 {font_obj} 0 R >> >> /Contents {} 0 R >>",
            4 + i * 2
        ));
        let c = content(lines);
        objs.push(format!("<< /Length {} >>\nstream\n{}endstream", c.len(), c));
    }
    objs.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());

    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
    }
    let xref_pos = out.len();
    let mut xref = format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1);
    for off in &offsets {
        xref.push_str(&format!("{off:010} 00000 n \n"));
    }
    out.extend_from_slice(xref.as_bytes());
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn test_born_digital_pdf_gets_native_coordinates_end_to_end() {
    // The default policy must route a real born-digital PDF through the native
    // extractor: the document lands with coord_space = "pdf-native", spatial rows
    // AND doc_chunk_blocks rows (the sentence layer's bbox source) for every chunk.
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("born-digital.pdf");
    fs::write(
        &pdf,
        minimal_born_digital_pdf(&[
            &[
                "Native extraction test paragraph one, discussing energeia.",
                "It continues on a second line for the same paragraph.",
            ],
            &["Second page body text about phantasma and position."],
        ]),
    )
    .unwrap();
    let db = test_db();
    let result = ingest_file(&db, &pdf).await.unwrap();

    assert_eq!(
        result.pdf_coord,
        Some("pdf-native"),
        "born-digital routing must select the native extractor (warnings: {:?})",
        result.warnings
    );
    let chunks = store::list_chunks_for_doc(&db, &result.document_id).unwrap();
    assert!(!chunks.is_empty());
    let joined = chunks
        .iter()
        .map(|c| c.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("energeia"), "page 1 text present");
    assert!(joined.contains("phantasma"), "page 2 text present");

    for chunk in &chunks {
        let sp = store::get_chunk_spatial(&db, &chunk.chunk_id)
            .unwrap()
            .expect("every chunk carries a spatial row on the native path");
        assert_eq!(sp.coord_space, "pdf-native");
        assert_ne!(sp.super_box, "[0.0,0.0,0.0,0.0]", "real, non-sentinel bbox");
    }
    let block_rows = store::list_chunk_blocks_for_doc(&db, &result.document_id).unwrap();
    assert!(
        block_rows.values().any(|rows| !rows.is_empty()),
        "doc_chunk_blocks written (sentence-tight bbox source)"
    );
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn test_native_extractor_disabled_falls_back_to_flat_text() {
    // Kill switch: use_pdf_native_extractor = false must reproduce the pre-native
    // behavior exactly — flat text, COORD_NONE, no spatial rows.
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("born-digital-off.pdf");
    fs::write(
        &pdf,
        minimal_born_digital_pdf(&[&["Flat fallback body text when native is off."]]),
    )
    .unwrap();
    let db = test_db();
    let mut policy = archon_policy::EffectivePolicy::default();
    policy.docs.pdf.use_pdf_native_extractor = false;
    let result = ingest_file_with_policy(&db, &pdf, &policy).await.unwrap();

    assert_eq!(
        result.pdf_coord,
        Some("none"),
        "kill switch restores COORD_NONE"
    );
    let chunks = store::list_chunks_for_doc(&db, &result.document_id).unwrap();
    assert!(!chunks.is_empty());
    assert!(chunks.iter().any(|c| c.content.contains("Flat fallback")));
    for chunk in &chunks {
        assert!(
            store::get_chunk_spatial(&db, &chunk.chunk_id)
                .unwrap()
                .is_none(),
            "flat path writes no spatial rows"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn test_native_text_matches_flat_pdftotext() {
    // Regression gate (plan Phase 8): the native path must not change WHAT text is
    // ingested — only add coordinates. Ingest the same PDF with native on and off and
    // compare the token SEQUENCE (order-sensitive: a multiset comparison could not
    // catch reading-order regressions). This fixture has no running heads, so the
    // sequences must match exactly; corpus-scale parity (99.5%+ net of intentional
    // header/watermark stripping) is measured against real PDFs at reprocess time.
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("parity.pdf");
    fs::write(
        &pdf,
        minimal_born_digital_pdf(&[
            &[
                "Alpha paragraph one has several plain words to compare.",
                "Beta continues the first page with more prose content.",
            ],
            &["Gamma opens the second page before the document ends."],
        ]),
    )
    .unwrap();

    let toks = |joined: &str| {
        joined
            .split_whitespace()
            .map(|t| {
                t.chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>()
                    .to_lowercase()
            })
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
    };

    let db_native = test_db();
    let native = ingest_file(&db_native, &pdf).await.unwrap();
    assert_eq!(native.pdf_coord, Some("pdf-native"));
    let native_text = store::list_chunks_for_doc(&db_native, &native.document_id)
        .unwrap()
        .iter()
        .map(|c| c.content.clone())
        .collect::<Vec<_>>()
        .join("\n");

    let db_flat = test_db();
    let mut policy = archon_policy::EffectivePolicy::default();
    policy.docs.pdf.use_pdf_native_extractor = false;
    let flat = ingest_file_with_policy(&db_flat, &pdf, &policy)
        .await
        .unwrap();
    assert_eq!(flat.pdf_coord, Some("none"));
    let flat_text = store::list_chunks_for_doc(&db_flat, &flat.document_id)
        .unwrap()
        .iter()
        .map(|c| c.content.clone())
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        toks(&native_text),
        toks(&flat_text),
        "native path changed the ingested token sequence"
    );
}
