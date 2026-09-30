//! Review findings under the authored run's terminal rule.
//!
//! The finding lists read here are the ones `validate_review_accounting_from_reducers`
//! already held equal to what the host attached to the final reducers, so
//! they are host data. Task attribution uses the host's own reader
//! (`review_findings::task_ids_of`, which the host also normalises every
//! attached finding to), restricted to universe tasks.
//!
//! Batch O: whatever a finding names and whatever severity a reviewer gave
//! it, it holds the run until a verifier's own verdict on it closes it
//! (`v3_run_outcome_closure`); the host's remediation plan routes the ones
//! that name no task, so none is merely listed.
//!
//! A finding the host marked `review_outcome: unreviewed` holds the run
//! whatever it names and whatever remediation reports: it records a review
//! that never completed, and only a completed review clears that.

use std::collections::BTreeSet;

use super::keys::TaskKeys;
use super::{AuthoredCallFact, UNREVIEWED_REVIEW_OUTCOME, Verdict, array, text};
use crate::v2::review_findings::task_ids_of;

/// Batch O: a finding the host marked unreviewed holds the run whatever it
/// names; every other finding, at any severity and whoever it names, must
/// be closed by a verifier's own verdict on it (`closure`), which also
/// decides the blocked tasks.
pub(super) fn check_findings(
    accounting: &serde_json::Value,
    calls: &[AuthoredCallFact],
    keys: &TaskKeys<'_>,
    discharged: &BTreeSet<String>,
    v: &mut Verdict,
) {
    for field in ["adversarial_findings", "uncovered_requirements"] {
        for finding in array(accounting.get(field)) {
            // The host's record of a review that never completed: no
            // writer's change supplies a missing verdict.
            if text(finding.get("review_outcome")) == UNREVIEWED_REVIEW_OUTCOME {
                let tasks: Vec<String> = task_ids_of(finding)
                    .iter()
                    .filter_map(|id| keys.task(id))
                    .collect();
                let label = finding_label(finding);
                let named = if tasks.is_empty() {
                    label
                } else {
                    format!("{label} ({})", tasks.join(", "))
                };
                v.block(format!("the host recorded {named} as unreviewed"), false);
            }
        }
    }
    super::closure::check_finding_closure(accounting, calls, keys, discharged, v);
}

/// A short human label: an id if the finding has one, else its text.
pub(super) fn finding_label(finding: &serde_json::Value) -> String {
    const KEYS: [&str; 7] = [
        "id",
        "finding_id",
        "requirement_id",
        "claim",
        "title",
        "summary",
        "finding",
    ];
    let named = KEYS.iter().find_map(|key| {
        finding
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    });
    match (named, finding.as_str()) {
        (Some(text), _) | (None, Some(text)) => format!("`{}`", clip_label(text)),
        (None, None) => format!("`{}`", clip_label(&finding.to_string())),
    }
}

fn clip_label(text: &str) -> String {
    const LIMIT: usize = 120;
    if text.chars().count() <= LIMIT {
        return text.to_string();
    }
    format!("{}...", text.chars().take(LIMIT).collect::<String>())
}
