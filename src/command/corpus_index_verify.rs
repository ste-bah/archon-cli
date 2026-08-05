//! Tier-2 mandatory verification (2026-08-05): the `--verify-quotes` gate for
//! `corpus-index import clauses`.
//!
//! Every ANCHORED incoming clause (one carrying an `archon:<doc>` text_layer_id)
//! must locate its quote inside that document — exact match, or fuzzy similarity
//! >= 0.90 (the house match_kind gate) — or the record is QUARANTINED instead of
//! written. Entries are born verified; the post-hoc stale-row sweeps of 2026-08-04
//! become structurally unnecessary. Unanchored rows (empty text_layer_id — the
//! cited-only class) and non-archon refs skip the gate by design.

use archon_docs::quote_verify::{self, MatchKind};
use cozo::DbInstance;

pub(crate) const FUZZY_FLOOR: f64 = 0.90;

#[derive(Default)]
pub(crate) struct QuoteGateStats {
    pub exact: usize,
    pub fuzzy: usize,
    pub skipped_unanchored: usize,
    pub rejected: usize,
}

impl QuoteGateStats {
    pub fn summary(&self) -> String {
        format!(
            "quote gate: {} exact, {} fuzzy>={FUZZY_FLOOR}, {} unanchored (skipped), {} rejected",
            self.exact, self.fuzzy, self.skipped_unanchored, self.rejected
        )
    }
}

/// A record that passed (or skipped) the gate: (source line, record).
pub(crate) type PassedRecord = (usize, serde_json::Value);
/// A record the gate rejected: (source line, record, human-readable reason).
pub(crate) type RejectedRecord = (usize, serde_json::Value, String);

/// Partition clause records into (passing, rejected-with-reason). Passing includes
/// unanchored rows; rejected rows carry a human-readable reason for the quarantine
/// sidecar. Never mutates records.
pub(crate) fn gate_clause_records(
    db: &DbInstance,
    records: Vec<PassedRecord>,
) -> (Vec<PassedRecord>, Vec<RejectedRecord>, QuoteGateStats) {
    let mut pass = Vec::new();
    let mut reject = Vec::new();
    let mut stats = QuoteGateStats::default();

    for (line, obj) in records {
        let doc = obj
            .get("text_layer_id")
            .and_then(|v| v.as_str())
            .and_then(|s| s.strip_prefix("archon:"))
            .map(str::to_string);
        let Some(doc_id) = doc.filter(|d| d.starts_with("doc-")) else {
            stats.skipped_unanchored += 1;
            pass.push((line, obj));
            continue;
        };
        let quote = obj
            .get("normalized_quote")
            .and_then(|v| v.as_str())
            .filter(|q| !q.trim().is_empty())
            .or_else(|| obj.get("quote").and_then(|v| v.as_str()))
            .unwrap_or("")
            .trim()
            .to_string();
        if quote.split_whitespace().count() < 4 {
            stats.rejected += 1;
            reject.push((line, obj, "anchored row with <4-word quote".into()));
            continue;
        }
        match quote_verify::find_fragment_bboxes(db, &doc_id, &quote) {
            Ok(Some(loc)) if loc.match_kind == MatchKind::Exact => {
                stats.exact += 1;
                pass.push((line, obj));
            }
            Ok(Some(loc)) if loc.similarity >= FUZZY_FLOOR => {
                stats.fuzzy += 1;
                pass.push((line, obj));
            }
            Ok(Some(loc)) => {
                stats.rejected += 1;
                reject.push((
                    line,
                    obj,
                    format!(
                        "quote verifies below threshold in {doc_id} \
                         (similarity {:.2} < {FUZZY_FLOOR})",
                        loc.similarity
                    ),
                ));
            }
            Ok(None) => {
                stats.rejected += 1;
                reject.push((line, obj, format!("quote not found in {doc_id}")));
            }
            Err(e) => {
                stats.rejected += 1;
                reject.push((line, obj, format!("verify-quote error for {doc_id}: {e}")));
            }
        }
    }
    (pass, reject, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_docs::models::ChunkArtifact;

    fn db_with_doc_chunk() -> DbInstance {
        let db = DbInstance::new("mem", "", "").unwrap();
        archon_docs::schema::ensure_doc_schema(&db).unwrap();
        archon_docs::store::insert_chunk(
            &db,
            &ChunkArtifact {
                chunk_id: "chunk-doc-aaaa-0".into(),
                document_id: "doc-aaaa".into(),
                artifact_id: "ocr-aaaa".into(),
                chunk_index: 0,
                page_start: 1,
                page_end: 1,
                content: "The discriminating power and the time of its exercise \
                          must be one and undivided."
                    .into(),
                content_hash: "h".into(),
                embedding_status: "pending".into(),
            },
        )
        .unwrap();
        db
    }

    fn rec(text_layer: &str, quote: &str) -> (usize, serde_json::Value) {
        (
            1,
            serde_json::json!({
                "clause_id": "cl-test-001",
                "text_layer_id": text_layer,
                "normalized_quote": quote,
            }),
        )
    }

    #[test]
    fn exact_quote_passes_the_gate() {
        let db = db_with_doc_chunk();
        let (pass, reject, stats) = gate_clause_records(
            &db,
            vec![rec(
                "archon:doc-aaaa",
                "the time of its exercise must be one and undivided",
            )],
        );
        assert_eq!(pass.len(), 1, "reject: {reject:?}");
        assert_eq!(stats.exact + stats.fuzzy, 1);
    }

    #[test]
    fn fabricated_quote_is_quarantined() {
        let db = db_with_doc_chunk();
        let (pass, reject, stats) = gate_clause_records(
            &db,
            vec![rec(
                "archon:doc-aaaa",
                "phlogiston chambers definitely orbit the crystalline moon tonight",
            )],
        );
        assert!(pass.is_empty());
        assert_eq!(reject.len(), 1);
        assert_eq!(stats.rejected, 1);
        assert!(reject[0].2.contains("doc-aaaa"), "reason: {}", reject[0].2);
    }

    #[test]
    fn unanchored_row_skips_the_gate() {
        let db = db_with_doc_chunk();
        let (pass, reject, stats) =
            gate_clause_records(&db, vec![rec("", "any quote about anything at all here")]);
        assert_eq!(pass.len(), 1);
        assert!(reject.is_empty());
        assert_eq!(stats.skipped_unanchored, 1);
    }

    #[test]
    fn anchored_row_with_tiny_quote_is_quarantined() {
        let db = db_with_doc_chunk();
        let (pass, reject, _stats) =
            gate_clause_records(&db, vec![rec("archon:doc-aaaa", "one two three")]);
        assert!(pass.is_empty());
        assert!(reject[0].2.contains("<4-word"));
    }
}
