//! Inline admissibility gate (index-overhaul Phase E → "runs on every ingest").
//!
//! The E0 corpus probes measure damage after the fact; this module runs the
//! same checks per document INSIDE the ingest, before the Ingested status flip,
//! so a damaged extraction can never be admitted silently:
//!
//!   FAIL  — ligature dropout markers in chunk text (the pypdfium2 4.30.0 class:
//!           `beneficial`→`benefcial`; citation-fatal, invisible to later checks);
//!   FAIL  — zero chunks for a text-bearing media type (degenerate extraction);
//!   FAIL  — chunks exist but the sentence layer is empty (invariant breach);
//!   FAIL  — zero spatial rows for a PDF (the bbox-less Marker-fallback class:
//!           whole-doc GPU OOM → CPU timeout → text-chunk fallback silently
//!           zeroed B&T's 99.5% spatial layer, F1 2026-07-31 — never "ok");
//!   WARN  — duplicate page-text hashes (the audited identical-pages class);
//!   WARN  — known OCR-damage tokens.
//!
//! `ARCHON_ADMISSIBILITY=warn` downgrades failures to warnings; `off` skips the
//! gate. Default is enforcing.

use cozo::{DbInstance, ScriptMutability};

use crate::errors::DocsError;

/// Fragments that only occur when pypdfium2 drops fi/ff/fl ligatures. Kept in
/// sync with the E0 probe list in `corpus-index probes`.
const LIGATURE_MARKERS: &[&str] = &[
    "benefcial",
    "signifcant",
    "difcult",
    "specifcally",
    "confdence",
    "benefts",
    "signifcance",
    "efcient",
    "infuence",
    "refect ",
    "frst ",
    "fnd ",
];

/// Known shredded-OCR tokens from the audited corpus (warning-class).
const OCR_DAMAGE_MARKERS: &[&str] = &["pnractical", "consiaered", "Involvernent", "1tsely"];

#[derive(Clone, Debug, Default)]
pub struct AdmissibilityReport {
    pub chunks: usize,
    pub sentences: usize,
    pub spatial_chunks: usize,
    pub ligature_hits: usize,
    pub ocr_damage_hits: usize,
    pub failures: Vec<String>,
    pub warnings: Vec<String>,
}

enum Mode {
    Enforce,
    Warn,
    Off,
}

fn mode() -> Mode {
    match std::env::var("ARCHON_ADMISSIBILITY")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "off" | "0" => Mode::Off,
        "warn" => Mode::Warn,
        _ => Mode::Enforce,
    }
}

fn storage(e: impl std::fmt::Display) -> DocsError {
    DocsError::Storage {
        message: e.to_string(),
    }
}

/// Run the admissibility checks for one freshly ingested document.
/// `expect_text` is false for pure-image media (empty OCR is legitimate there).
/// `expect_spatial` is true for PDFs: every PDF pipeline (Marker or block-layer)
/// produces spatial rows, so a PDF with chunks but ZERO spatial rows is the
/// bbox-less fallback signature and must not be admitted as ok.
/// Returns the report; `report.failures` is non-empty when the document must
/// NOT be admitted (already downgraded to warnings in `warn` mode).
pub fn check_document(
    db: &DbInstance,
    document_id: &str,
    expect_text: bool,
    expect_spatial: bool,
) -> Result<AdmissibilityReport, DocsError> {
    let mut report = AdmissibilityReport::default();
    let mode = mode();
    if matches!(mode, Mode::Off) {
        return Ok(report);
    }

    let param = |v: &str| {
        let mut p = std::collections::BTreeMap::new();
        p.insert("d".to_string(), cozo::DataValue::from(v));
        p
    };

    let chunks = db
        .run_script(
            "?[chunk_id, content] := *doc_chunks{chunk_id, document_id, content}, document_id = $d",
            param(document_id),
            ScriptMutability::Immutable,
        )
        .map_err(storage)?;
    report.chunks = chunks.rows.len();
    for row in &chunks.rows {
        let content = row[1].get_str().unwrap_or_default();
        report.ligature_hits += LIGATURE_MARKERS
            .iter()
            .map(|m| content.matches(m).count())
            .sum::<usize>();
        report.ocr_damage_hits += OCR_DAMAGE_MARKERS
            .iter()
            .map(|m| content.matches(m).count())
            .sum::<usize>();
    }

    let sentences = db
        .run_script(
            "?[count(sentence_idx)] := *doc_chunks{chunk_id, document_id}, document_id = $d, \
             *doc_chunk_sentences{chunk_id, sentence_idx}",
            param(document_id),
            ScriptMutability::Immutable,
        )
        .map_err(storage)?;
    report.sentences = sentences
        .rows
        .first()
        .and_then(|r| r[0].get_int())
        .unwrap_or(0) as usize;

    let spatial = db
        .run_script(
            "?[count(chunk_id)] := *doc_chunks{chunk_id, document_id}, document_id = $d, \
             *doc_chunk_spatial{chunk_id}",
            param(document_id),
            ScriptMutability::Immutable,
        )
        .map_err(storage)?;
    report.spatial_chunks = spatial
        .rows
        .first()
        .and_then(|r| r[0].get_int())
        .unwrap_or(0) as usize;

    let pages = db
        .run_script(
            "?[text_hash] := *doc_pages{page_id, document_id, text_hash}, document_id = $d",
            param(document_id),
            ScriptMutability::Immutable,
        )
        .map_err(storage)?;
    let real_hashes: Vec<&str> = pages
        .rows
        .iter()
        .filter_map(|r| r[0].get_str())
        .filter(|h| !h.is_empty() && *h != "none")
        .collect();
    let distinct: std::collections::HashSet<&&str> = real_hashes.iter().collect();

    if report.ligature_hits > 0 {
        report.failures.push(format!(
            "ligature dropout: {} marker hit(s) in chunk text — pypdfium2 fi/ff/fl \
             regression (citation-fatal); check the marker venv pin",
            report.ligature_hits
        ));
    }
    if expect_text && report.chunks == 0 {
        report
            .failures
            .push("degenerate extraction: 0 chunks for a text-bearing media type".into());
    }
    if report.chunks > 0 && report.sentences == 0 {
        report
            .failures
            .push("sentence layer empty despite chunks — Ingested ⇒ sentences invariant".into());
    }
    if expect_spatial && report.chunks > 0 && report.spatial_chunks == 0 {
        report.failures.push(format!(
            "bbox-less fallback extraction: 0 of {} chunks carry spatial rows for a PDF — \
             the Marker fallback zeroed the spatial layer; re-queue under a windowed \
             Marker budget (marker_memory_budget_mb) instead of admitting",
            report.chunks
        ));
    }
    if real_hashes.len() >= 3 && distinct.len() < real_hashes.len() {
        report.warnings.push(format!(
            "duplicate page-text hashes: {} pages with text, only {} distinct",
            real_hashes.len(),
            distinct.len()
        ));
    }
    if report.ocr_damage_hits > 0 {
        report.warnings.push(format!(
            "known OCR-damage tokens: {} hit(s)",
            report.ocr_damage_hits
        ));
    }

    if matches!(mode, Mode::Warn) && !report.failures.is_empty() {
        let downgraded = report
            .failures
            .drain(..)
            .map(|f| format!("ADMISSIBILITY (downgraded): {f}"));
        report.warnings.extend(downgraded);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest_text::ingest_text_source;
    use crate::schema::ensure_doc_schema;

    fn test_db() -> DbInstance {
        let db = DbInstance::new("mem", "", "").unwrap();
        ensure_doc_schema(&db).unwrap();
        db
    }

    #[test]
    fn clean_document_is_admissible() {
        let db = test_db();
        let r = ingest_text_source(
            &db,
            "corpus/test/clean.txt",
            "text/plain",
            "A perfectly ordinary sentence about significant findings. Another one follows it.",
        )
        .unwrap();
        let report = check_document(&db, &r.document_id, true, false).unwrap();
        assert!(
            report.failures.is_empty(),
            "failures: {:?}",
            report.failures
        );
        assert!(report.chunks > 0 && report.sentences > 0);
    }

    #[test]
    fn ligature_dropout_is_a_failure() {
        let db = test_db();
        let r = ingest_text_source(
            &db,
            "corpus/test/damaged.txt",
            "text/plain",
            "The results were benefcial and signifcant across every trial we ran.",
        )
        .unwrap();
        let report = check_document(&db, &r.document_id, true, false).unwrap();
        assert_eq!(report.ligature_hits, 2);
        assert!(
            report.failures.iter().any(|f| f.contains("ligature")),
            "expected ligature failure, got {:?}",
            report.failures
        );
    }

    #[test]
    fn missing_document_reports_no_chunks() {
        let db = test_db();
        let report = check_document(&db, "doc-does-not-exist", true, false).unwrap();
        assert_eq!(report.chunks, 0);
        assert!(report.failures.iter().any(|f| f.contains("degenerate")));
    }

    #[test]
    fn bboxless_fallback_on_pdf_is_a_failure() {
        // The B&T/FCM class (F1 2026-07-31): chunks + sentences present but zero
        // spatial rows on a document whose pipeline should have produced them.
        let db = test_db();
        let r = ingest_text_source(
            &db,
            "corpus/test/fallback.pdf",
            "text/plain",
            "A chapter of perfectly extractable prose. It reads fine and hides the loss.",
        )
        .unwrap();
        let report = check_document(&db, &r.document_id, true, true).unwrap();
        assert_eq!(report.spatial_chunks, 0);
        assert!(
            report.failures.iter().any(|f| f.contains("bbox-less")),
            "expected bbox-less fallback failure, got {:?}",
            report.failures
        );
    }

    #[test]
    fn pdf_with_spatial_rows_is_admissible() {
        let db = test_db();
        let r = ingest_text_source(
            &db,
            "corpus/test/spatial.pdf",
            "text/plain",
            "A chapter of perfectly extractable prose. It reads fine and keeps its boxes.",
        )
        .unwrap();
        // give the first chunk a spatial row, as the Marker/block path would
        let chunks = db
            .run_script(
                "?[chunk_id] := *doc_chunks{chunk_id, document_id}, document_id = $d",
                {
                    let mut p = std::collections::BTreeMap::new();
                    p.insert(
                        "d".to_string(),
                        cozo::DataValue::from(r.document_id.as_str()),
                    );
                    p
                },
                ScriptMutability::Immutable,
            )
            .unwrap();
        let chunk_id = chunks.rows[0][0].get_str().unwrap().to_string();
        db.run_script(
            "?[chunk_id, page_num, super_box, blocks, coord_space, spatial_hash] <- \
             [[$c, 1, '[]', '[]', 'pdf', 'h']] \
             :put doc_chunk_spatial { chunk_id => page_num, super_box, blocks, coord_space, spatial_hash }",
            {
                let mut p = std::collections::BTreeMap::new();
                p.insert("c".to_string(), cozo::DataValue::from(chunk_id.as_str()));
                p
            },
            ScriptMutability::Mutable,
        )
        .unwrap();
        let report = check_document(&db, &r.document_id, true, true).unwrap();
        assert!(report.spatial_chunks > 0);
        assert!(
            !report.failures.iter().any(|f| f.contains("bbox-less")),
            "spatial layer present must not trip the fallback gate: {:?}",
            report.failures
        );
    }
}
