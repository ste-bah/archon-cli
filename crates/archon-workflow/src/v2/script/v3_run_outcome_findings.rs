//! Review findings under the authored run's terminal rule.
//!
//! The finding lists read here are the ones `validate_review_accounting_from_reducers`
//! already held equal to what the host attached to the final reducers, so
//! they are host data. Task attribution uses the host's own reader
//! (`review_findings::task_ids_of`, which the host also normalises every
//! attached finding to), restricted to universe tasks.
//!
//! A finding that names universe tasks but opts out of single-task
//! attribution (`attributable_to_task: false`) is remediated across all of
//! them by the prelude, under the cross-task key; only a finding naming no
//! universe task is unassigned.
//!
//! A finding the host marked `review_outcome: unreviewed` holds the run
//! whatever it names and whatever remediation reports: it records a review
//! that never completed, and only a completed review clears that.

use std::collections::BTreeSet;

use super::keys::{TaskKeys, cross_key};
use super::{UNREVIEWED_REVIEW_OUTCOME, Verdict, array, text};
use crate::v2::review_findings::task_ids_of;

/// The only severities an unassigned finding may carry without holding the
/// run. Anything else — including a missing or unknown severity — blocks.
const NON_BLOCKING_SEVERITIES: [&str; 7] = [
    "low",
    "info",
    "informational",
    "note",
    "minor",
    "trivial",
    "nit",
];

/// Every remediation key the findings call for needs an outcome
/// (`outcomes`); an unassigned finding blocks unless it is a low-impact
/// adversarial note, which is listed instead.
pub(super) fn check_findings(
    accounting: &serde_json::Value,
    outcomes: &BTreeSet<String>,
    keys: &TaskKeys<'_>,
    v: &mut Verdict,
) {
    let mut needed = BTreeSet::new();
    let mut listed = Vec::new();
    for (field, coverage) in [
        ("adversarial_findings", false),
        ("uncovered_requirements", true),
    ] {
        for finding in array(accounting.get(field)) {
            let tasks: Vec<String> = task_ids_of(finding)
                .iter()
                .filter_map(|id| keys.task(id))
                .collect();
            // The host's record of a review that never completed. It names
            // the task it was reviewing and opts out of single-task
            // attribution, but it is no cross-task defect: no writer's change
            // supplies a missing verdict, so no remediation outcome clears it.
            if text(finding.get("review_outcome")) == UNREVIEWED_REVIEW_OUTCOME {
                let label = finding_label(finding);
                let named = if tasks.is_empty() {
                    label
                } else {
                    format!("{label} ({})", tasks.join(", "))
                };
                v.block(format!("the host recorded {named} as unreviewed"), false);
                continue;
            }
            if !tasks.is_empty() {
                if finding.get("attributable_to_task") == Some(&serde_json::Value::Bool(false)) {
                    needed.insert(cross_key(tasks));
                } else {
                    needed.extend(tasks);
                }
                continue;
            }
            let label = finding_label(finding);
            let severity = severity(finding);
            if coverage {
                v.block(
                    format!("uncovered requirement {label} names no task, so nothing covers it"),
                    false,
                );
            } else if NON_BLOCKING_SEVERITIES.contains(&severity.as_str()) {
                listed.push(label);
            } else {
                let shown = if severity.is_empty() {
                    "no".to_string()
                } else {
                    format!("`{severity}`")
                };
                v.block(
                    format!("finding {label} names no task and has {shown} severity"),
                    false,
                );
            }
        }
    }
    for key in needed.difference(outcomes) {
        v.block(
            format!(
                "review findings name task {key} but review remediation reports no outcome for it"
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

/// Lower-cased, trimmed severity; empty when absent or not a string.
fn severity(finding: &serde_json::Value) -> String {
    text(finding.get("severity")).trim().to_ascii_lowercase()
}

/// A short human label: an id if the finding has one, else its text.
fn finding_label(finding: &serde_json::Value) -> String {
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
