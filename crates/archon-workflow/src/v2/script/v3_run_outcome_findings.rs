//! Review findings under the authored run's terminal rule.
//!
//! The finding lists read here are the ones `validate_review_accounting_from_reducers`
//! already held equal to what the host attached to the final reducers, so
//! they are host data. Task attribution uses the host's own reader
//! (`review_findings::task_ids_of`), plus the prelude's explicit
//! `attributable_to_task: false` opt-out.

use std::collections::BTreeSet;

use super::{UNREVIEWED_REVIEW_OUTCOME, Verdict, array, clip, text};
use crate::v2::review_findings::task_ids_of;

/// Severities a finding naming no task cannot be waved through at.
const BLOCKING_SEVERITIES: [&str; 3] = ["high", "critical", "blocking"];

/// Every task a finding names needs a remediation outcome (`outcomes`); a
/// finding naming no task blocks when it is an uncovered requirement, a host
/// `unreviewed` marker, or of blocking severity, and is listed otherwise.
pub(super) fn check_findings(
    accounting: &serde_json::Value,
    outcomes: &BTreeSet<String>,
    v: &mut Verdict,
) {
    let mut named = BTreeSet::new();
    let mut listed = Vec::new();
    for (field, coverage) in [
        ("adversarial_findings", false),
        ("uncovered_requirements", true),
    ] {
        for finding in array(accounting.get(field)) {
            let tasks = attributed_tasks(finding);
            if !tasks.is_empty() {
                named.extend(tasks);
                continue;
            }
            let label = finding_label(finding);
            if coverage {
                v.block(
                    format!("uncovered requirement {label} names no task, so nothing covers it"),
                    false,
                );
            } else if text(finding.get("review_outcome")) == UNREVIEWED_REVIEW_OUTCOME {
                v.block(format!("the host recorded {label} as unreviewed"), false);
            } else if BLOCKING_SEVERITIES.contains(&severity(finding).as_str()) {
                v.block(
                    format!("{} finding {label} names no task", severity(finding)),
                    false,
                );
            } else {
                listed.push(label);
            }
        }
    }
    for task in named.difference(outcomes) {
        v.block(
            format!(
                "review findings name task {task} but review remediation reports no outcome for it"
            ),
            false,
        );
    }
    if !listed.is_empty() {
        v.notes.push(format!(
            "{} non-blocking finding(s) name no task: {}",
            listed.len(),
            listed.join(", ")
        ));
    }
}

/// The tasks a finding is charged to: none when it opts out explicitly.
pub(super) fn attributed_tasks(finding: &serde_json::Value) -> Vec<String> {
    if finding.get("attributable_to_task") == Some(&serde_json::Value::Bool(false)) {
        return Vec::new();
    }
    task_ids_of(finding)
}

fn severity(finding: &serde_json::Value) -> String {
    text(finding.get("severity")).trim().to_ascii_lowercase()
}

fn finding_label(finding: &serde_json::Value) -> String {
    let id = ["id", "finding_id", "requirement_id"]
        .iter()
        .find_map(|key| finding.get(*key).and_then(serde_json::Value::as_str));
    match (id, finding.as_str()) {
        (Some(id), _) => format!("`{}`", clip(id)),
        (None, Some(bare)) => format!("`{}`", clip(bare)),
        (None, None) => format!("`{}`", clip(&finding.to_string())),
    }
}
