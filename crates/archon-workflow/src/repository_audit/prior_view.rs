//! Bounded prior view for the assessor prompt (Issue-23).
//!
//! The ledger stores every past report in full; the prompt must not. Live,
//! after 20 audits in one run the inlined ledger reached 677 KB (~170k tokens)
//! and grew ~10k tokens per wave. This view carries one record per declared
//! path (its most recent judgment, reason clipped) plus the small bookkeeping
//! collections the assessor keys on. The ledger itself is untouched.
use super::{
    RequiredAction, Verdict,
    correction::Correction,
    ledger::{AuditLedger, Obligation, Reassessment, Waiver},
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Maximum bytes of `reason` carried per record before clipping.
pub const REASON_LIMIT: usize = 400;
/// Appended to a clipped reason so the assessor knows text was elided.
pub const CLIP_MARKER: &str = "…";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PriorRecord {
    pub declared_path: String,
    pub verdict: Verdict,
    pub equivalents: Vec<String>,
    pub required_action: RequiredAction,
    pub snapshot: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PriorView<'a> {
    pub history_reports: usize,
    pub records: Vec<PriorRecord>,
    pub obligations: &'a BTreeMap<String, Obligation>,
    pub waivers: &'a [Waiver],
    pub reassessments: &'a [Reassessment],
    pub corrections: &'a [Correction],
}

impl<'a> PriorView<'a> {
    /// One record per declared path from the last report that mentions it.
    /// History is append-only, so the scan runs from the newest report back
    /// and stops once every declared path has been seen. Paths never assessed
    /// are absent.
    pub fn build(ledger: &'a AuditLedger, declared_paths: &BTreeSet<String>) -> Self {
        let mut pending: BTreeSet<&str> = declared_paths.iter().map(String::as_str).collect();
        let mut found: BTreeMap<&str, PriorRecord> = BTreeMap::new();
        for report in ledger.history.iter().rev() {
            if pending.is_empty() {
                break;
            }
            for record in &report.records {
                if !pending.remove(record.declared_path.as_str()) {
                    continue;
                }
                found.insert(
                    record.declared_path.as_str(),
                    PriorRecord {
                        declared_path: record.declared_path.clone(),
                        verdict: record.verdict,
                        equivalents: record.equivalents.clone(),
                        required_action: record.required_action,
                        snapshot: report.snapshot.clone(),
                        reason: clip_reason(&record.reason),
                    },
                );
            }
        }
        Self {
            history_reports: ledger.history.len(),
            records: found.into_values().collect(),
            obligations: &ledger.obligations,
            waivers: &ledger.waivers,
            reassessments: &ledger.reassessments,
            corrections: &ledger.corrections,
        }
    }
}

/// Clip to `REASON_LIMIT` bytes on a char boundary and append `CLIP_MARKER`.
pub fn clip_reason(reason: &str) -> String {
    if reason.len() <= REASON_LIMIT {
        return reason.to_string();
    }
    let mut cut = REASON_LIMIT;
    while !reason.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut clipped = String::with_capacity(cut + CLIP_MARKER.len());
    clipped.push_str(&reason[..cut]);
    clipped.push_str(CLIP_MARKER);
    clipped
}

#[cfg(test)]
#[path = "prior_view_tests.rs"]
mod tests;
