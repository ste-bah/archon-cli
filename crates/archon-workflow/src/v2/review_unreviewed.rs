//! A review that did not complete must never read as a clean one.
//!
//! A review map branch that failed — a timeout, an inactivity cut, a transport
//! failure, an output the host rejected — has no result, so it carries no
//! findings. Counted as findings, that is zero, which is exactly what a task
//! reviewed clean reports: the task flowed into acceptance as reviewed. The
//! roster did record the failed status, but nothing downstream read it.
//!
//! So the host gives every such branch a finding of its own, naming the task
//! it was reviewing, marked [`UNREVIEWED_OUTCOME`] under [`REVIEW_OUTCOME_KEY`]
//! and severity `blocking`. It travels with the review's other findings through
//! the reduce (map findings are carried through structurally) into the
//! accounting the script reports, and the host's attachment lists the
//! unreviewed tasks by id. A clean review adds nothing: no finding, no id.
//!
//! It is marked `attributable_to_task: false`. No change a task's writer makes
//! can supply a missing verdict, and a writer handed "no defect is known" lands
//! nothing and is then recorded as having refuted the finding — an unreviewed
//! task misreported as a disputed one. Unattributable findings are returned
//! untouched as `unassigned`, so the marker stays exactly what it is.
//!
//! Domain-agnostic: it reads branch ids, statuses, errors and the task ids the
//! host itself stamped on each branch input — nothing inside a finding.

use std::collections::BTreeMap;

use serde_json::Value;

use super::super::outcome_envelope::outcomes_of;
use super::{collect_findings, task_ids_of};

/// The finding field that says what kind of review outcome a finding records.
pub const REVIEW_OUTCOME_KEY: &str = "review_outcome";

/// The review outcome of a task whose review never completed.
pub const UNREVIEWED_OUTCOME: &str = "unreviewed";

/// The attachment field listing the tasks a review never completed for.
pub(super) const UNREVIEWED_TASK_IDS_KEY: &str = "unreviewed_task_ids";

const ERROR_EXCERPT_CHARS: usize = 400;

/// A branch view ended without a verdict: no result at all, or a failed,
/// blocked or cancelled branch that returned no finding. A branch that
/// returned findings reviewed its task, whatever status it reported.
fn review_incomplete(outcome: &Value) -> bool {
    let result = outcome.get("result").filter(|result| !result.is_null());
    if result.is_none() {
        return true;
    }
    let status = outcome
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(status.as_str(), "failed" | "blocked" | "cancelled")
        && collect_findings(outcome).is_empty()
}

/// The branch outcome views a map result carries. `outcomes_of` hands back
/// the value itself when it holds no branch array — a map that ran no branch
/// at all — and that value is not a branch; nor is any view without the
/// `item_id` and `status` every stored branch outcome has.
fn branch_views(map_result_data: &Value) -> Vec<Value> {
    outcomes_of(map_result_data)
        .into_iter()
        .filter(|view| view != map_result_data)
        .filter(|view| view.get("item_id").is_some() && view.get("status").is_some())
        .collect()
}

fn excerpt(text: &str) -> String {
    let mut out: String = text.chars().take(ERROR_EXCERPT_CHARS).collect();
    if text.chars().count() > ERROR_EXCERPT_CHARS {
        out.push_str("...");
    }
    out
}

/// One `unreviewed` finding for each branch of a review map that did not
/// complete, attributed to the task(s) the host built that branch for.
pub fn unreviewed_findings(
    map_result_data: &Value,
    item_task_ids: &BTreeMap<String, Vec<String>>,
    kind: &str,
) -> Vec<Value> {
    branch_views(map_result_data)
        .into_iter()
        .filter(review_incomplete)
        .map(|outcome| {
            let item_id = outcome
                .get("item_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let mut task_ids = item_task_ids.get(&item_id).cloned().unwrap_or_default();
            if task_ids.is_empty() {
                task_ids = task_ids_of(&outcome);
            }
            let status = outcome
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("failed");
            let error = outcome
                .get("error")
                .and_then(Value::as_str)
                .map(excerpt)
                .unwrap_or_else(|| "no result was recorded".to_string());
            let reviewed = if task_ids.is_empty() {
                "an unattributed task".to_string()
            } else {
                task_ids.join(", ")
            };
            serde_json::json!({
                "id": format!("unreviewed-{item_id}"),
                "canonical_task_ids": task_ids,
                "attributable_to_task": false,
                "severity": "blocking",
                REVIEW_OUTCOME_KEY: UNREVIEWED_OUTCOME,
                "review_kind": kind,
                "review_branch_id": item_id,
                "claim": format!(
                    "The {kind} review of {reviewed} did not complete (branch {item_id} \
                     {status}: {error}). No reviewer verdict exists: this task is \
                     UNREVIEWED, not reviewed clean."
                ),
                "remediation": "No defect is known and none was ruled out. Only a \
                     completed review of this task clears this: re-run its review.",
            })
        })
        .collect()
}

/// Is this finding the host's record of a review that did not complete?
pub fn is_unreviewed_finding(finding: &Value) -> bool {
    finding.get(REVIEW_OUTCOME_KEY).and_then(Value::as_str) == Some(UNREVIEWED_OUTCOME)
}

/// The tasks the given findings record as unreviewed, sorted and unique.
pub fn unreviewed_task_ids(findings: &[Value]) -> Vec<String> {
    let mut ids: Vec<String> = findings
        .iter()
        .filter(|finding| is_unreviewed_finding(finding))
        .flat_map(task_ids_of)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

#[cfg(test)]
#[path = "review_unreviewed_tests.rs"]
mod tests;
