// The narrowed terminal reduce (`cross-cutting-review`) and the structural
// merge of its output with the per-task adversarial findings.
//
// The old `adversarial-review` reduce was handed the whole run — task universe,
// implementation evidence, verification evidence, artifact evidence, learning
// context — and asked to review everything. Per-task review now owns that job,
// so this reduce is given ONLY what a per-task reviewer provably cannot see:
// the task universe (dependency edges, PRD-level acceptance) and a compact
// DIGEST of what the per-task reviewers already found. It cannot re-review a
// diff it was never shown.
//
// "Do not duplicate the per-task findings" is not left to the prompt.
// `merge_review` drops every cross-cutting item whose identity matches a
// per-task finding, so a reduce that restates them contributes nothing and the
// count of what it duplicated is recorded on the merged review.

use serde_json::Value;

use crate::generated_lifecycle_support as support;

use super::{adversarial, assignment_invalid};

/// Longest digest line kept per finding. A digest is for spotting
/// contradictions BETWEEN findings, not for re-adjudicating one.
const DIGEST_CLAIM_CHARS: usize = 240;

/// Narrow reduce input: task universe + finding digest. Deliberately excludes
/// implementation/verification/artifact evidence — that is per-task material.
pub fn cross_cutting_input(task_universe: &Value, per_task_findings: &[Value]) -> Value {
    serde_json::json!({
        "taskUniverse": task_universe,
        "perTaskFindingDigest": finding_digest(per_task_findings),
        "perTaskFindingCount": per_task_findings.len(),
        "scope": "cross_task_only",
    })
}

/// One compact line per per-task finding: which task, which finding, one claim.
fn finding_digest(findings: &[Value]) -> Vec<Value> {
    findings
        .iter()
        .map(|finding| {
            serde_json::json!({
                "canonical_task_ids": finding.get("canonical_task_ids"),
                "id": finding.get("id"),
                "severity": finding.get("severity"),
                "claim": claim_text(finding),
            })
        })
        .collect()
}

fn claim_text(finding: &Value) -> String {
    for key in ["claim", "title", "summary", "description", "message", "id"] {
        if let Some(value) = finding.get(key).and_then(Value::as_str) {
            let value = value.trim();
            if !value.is_empty() {
                return value.chars().take(DIGEST_CLAIM_CHARS).collect();
            }
        }
    }
    String::new()
}

/// The review value the remediation loop consumes.
///
/// Per-task findings are carried through VERBATIM by this function rather than
/// by asking the reducer to preserve them: the reduce cannot drop what it never
/// held. Cross-cutting items that restate a per-task finding are discarded.
///
/// Status is derived, not copied. `review_needs_remediation` short-circuits on
/// an `accepted` status, so a cross-cutting reduce that returns "accepted"
/// while per-task reviewers filed findings would have silently discarded every
/// one of them. Any surviving finding therefore forces `needs_remediation`.
pub fn merge_review(per_task_findings: &[Value], cross: &Value) -> Value {
    let mut identities: Vec<String> = Vec::new();
    for finding in per_task_findings {
        identities.extend(adversarial::finding_identities(finding));
    }
    let cross_items = support::array(cross.get("items"));
    let mut kept = Vec::new();
    let mut duplicates = 0usize;
    for item in cross_items {
        let is_duplicate = adversarial::finding_identities(&item)
            .iter()
            .any(|identity| identities.contains(identity));
        if is_duplicate {
            duplicates += 1;
            continue;
        }
        kept.push(mark_cross_cutting(item));
    }
    let mut items: Vec<Value> = per_task_findings.to_vec();
    let cross_cutting_kept = kept.len();
    items.extend(kept);
    // The third verdict is adjudicated here, over the SAME merged list, for the
    // same reason the status is derived here: a reviewer's word alone decides
    // nothing. Rejected claims come back downgraded and stay in `items`, so the
    // round loses no finding to a refused verdict.
    let admission = assignment_invalid::admit(&items);
    let items = admission.findings;
    // Fail-closed in three directions now. An admitted `assignment_invalid`
    // outranks remediation — it is not work to redo, it is work not to attempt
    // — so it takes the status and the driver stops the loop on it. Otherwise
    // surviving findings force remediation; an empty finding set accepts ONLY
    // if the cross-cutting reduce itself accepted. Defaulting an empty result
    // to "accepted" would have turned a reducer that transport-failed — which
    // returns no items because it never ran, not because it found nothing —
    // into a clean bill of health.
    let status = if !admission.admitted.is_empty() {
        assignment_invalid::VERDICT
    } else if !items.is_empty() {
        "needs_remediation"
    } else if support::outcome_accepted_or_noop(cross) {
        "accepted"
    } else {
        "cross_cutting_review_not_accepted"
    }
    .to_string();
    let mut merged = serde_json::Map::new();
    merged.insert("status".to_string(), Value::String(status));
    merged.insert(
        "summary".to_string(),
        Value::String(merged_summary(
            cross,
            per_task_findings.len(),
            cross_cutting_kept,
            duplicates,
        )),
    );
    merged.insert("items".to_string(), Value::Array(items));
    merged.insert(
        "per_task_finding_count".to_string(),
        serde_json::json!(per_task_findings.len()),
    );
    merged.insert(
        "cross_cutting_finding_count".to_string(),
        serde_json::json!(cross_cutting_kept),
    );
    merged.insert(
        "duplicate_cross_cutting_findings_dropped".to_string(),
        serde_json::json!(duplicates),
    );
    merged.insert(
        "assignment_invalid".to_string(),
        Value::Array(admission.admitted),
    );
    // Refused claims are recorded, not just downgraded. A reviewer that keeps
    // reaching for the verdict without evidence is itself a finding about the
    // run, and it is invisible if the only trace is a stripped field.
    merged.insert(
        "assignment_invalid_rejected".to_string(),
        Value::Array(admission.rejected),
    );
    merged.insert("cross_cutting_review".to_string(), cross.clone());
    Value::Object(merged)
}

fn mark_cross_cutting(item: Value) -> Value {
    let mut object = item.as_object().cloned().unwrap_or_default();
    if object.is_empty() {
        return item;
    }
    object.insert(
        "finding_scope".to_string(),
        Value::String("cross_cutting".to_string()),
    );
    Value::Object(object)
}

fn merged_summary(
    cross: &Value,
    per_task: usize,
    cross_cutting: usize,
    duplicates: usize,
) -> String {
    let base = cross
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or("cross-cutting review returned no summary");
    format!(
        "{base} | per-task findings: {per_task}; cross-cutting findings kept: {cross_cutting}; \
         cross-cutting items dropped as duplicates of per-task findings: {duplicates}"
    )
}

/// Canonical task ids named by a review-remediation inventory: the tasks whose
/// per-task review must be RE-RUN in the next review round. Re-reviewing only
/// what was remediated is what makes the review loop converge — stale findings
/// from a task nobody touched would otherwise re-enter every round until the
/// iteration cap.
pub fn remediated_task_ids(inventory: &Value) -> std::collections::BTreeSet<String> {
    support::array(inventory.get("items"))
        .iter()
        .flat_map(|item| support::strings_of(item.get("canonical_task_ids")))
        .collect()
}
