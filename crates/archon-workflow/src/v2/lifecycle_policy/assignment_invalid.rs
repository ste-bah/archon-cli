// The third review verdict: `assignment_invalid`.
//
// Review was two-valued. A round either produced findings to remediate or it
// accepted, and an agent whose assignment turns out to be mis-scoped — the task
// cannot be done as written, or should not be — had nowhere to put that. Its
// report came back as findings, the loop remediated them, the next round found
// the same thing again, and the six-round bound expired on a task nobody should
// have attempted. The lesson an agent draws from that shape is to produce
// SOMETHING that passes rather than to report the problem, which is the exact
// opposite of what review is for.
//
// So the verdict is first-class, and it is deliberately NOT remediable:
// `LifecycleDriver::block_assignment_invalid` ends the review loop on it and
// escalates, instead of spending the remaining rounds re-attempting work that
// should not be attempted at all.
//
// WHY THE VERDICT CANNOT SIMPLY BE CLAIMED
//
// A verdict that ends the loop is also an escape hatch from work an agent
// merely failed at, so admission is structural rather than advisory. Four
// conditions, and a claim missing any of them never becomes a verdict:
//
//  1. PROVENANCE. Only a per-task adversarial branch may raise it. Those
//     findings carry `attribution_source: "review_branch"`, stamped by
//     `adversarial::attributed_findings` from the HOST-stamped branch id — a
//     field no model writes. The cross-cutting reduce's items are stamped
//     `finding_scope: "cross_cutting"` by `merge_review` instead, so the one
//     agent that holds the whole run at once cannot declare any part of it
//     invalid. This is the same lever `merge_review` already uses to stop a
//     reducer claiming "accepted" over outstanding findings.
//  2. A NAMED TASK. `canonical_task_ids` must be non-empty. "Some assignment
//     somewhere is invalid" cannot be re-scoped by anyone.
//  3. A STATED REASON, at least [`MIN_REASON_CHARS`] of it. "invalid" is a
//     label, not a diagnosis.
//  4. FILE:LINE EVIDENCE. At least one `path:line` reference — the same bar
//     the task board puts on raising an item at all. An agent that has
//     established a task cannot be done can point at the code that makes it
//     so; an agent that merely failed at it cannot.
//
// A claim that misses any condition is DOWNGRADED, not dropped: the verdict
// keys are stripped, the admission failures are recorded on the finding, and it
// re-enters the round as an ordinary remediable finding. Dropping it would lose
// a real finding to a formatting mistake; honouring it would hand out the
// escape hatch to anyone who types the word.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::generated_lifecycle_support as support;

/// The verdict, spelled once. Reviewers are told this exact token by
/// `prompts::PER_TASK_ADVERSARIAL_REVIEW_TASK`, and `merge_review` promotes it
/// to the merged review's status.
pub const VERDICT: &str = "assignment_invalid";

/// Keys a reviewer may put a verdict under. Read, never required — a finding
/// without any of them is an ordinary finding.
const VERDICT_KEYS: &[&str] = &[
    "verdict",
    "review_verdict",
    "assignment_verdict",
    "outcome",
    "status",
];

/// Keys a stated reason may live under, best first.
const REASON_KEYS: &[&str] = &[
    "reason",
    "assignment_invalid_reason",
    "claim",
    "title",
    "summary",
    "description",
    "message",
];

/// Keys whose contents are scanned for `path:line` references.
const EVIDENCE_KEYS: &[&str] = &[
    "evidence",
    "counter_evidence",
    "evidence_refs",
    "references",
    "file_references",
    "files",
    "locations",
    "proof",
];

/// Shortest reason that can be a diagnosis rather than a label. One clause of
/// English about why the task is impossible clears this comfortably; the word
/// "invalid", or a restated task title, does not.
const MIN_REASON_CHARS: usize = 40;

/// Findings after admission, plus what admission decided.
///
/// `findings` is always the same length as the input: nothing is dropped here,
/// only downgraded, because a rejected claim is still a finding somebody made.
pub struct Admission {
    pub findings: Vec<Value>,
    pub admitted: Vec<Value>,
    pub rejected: Vec<Value>,
}

/// Adjudicate every `assignment_invalid` claim in a merged finding set.
pub fn admit(findings: &[Value]) -> Admission {
    let mut out = Admission {
        findings: Vec::with_capacity(findings.len()),
        admitted: Vec::new(),
        rejected: Vec::new(),
    };
    for finding in findings {
        if !claims_verdict(finding) {
            out.findings.push(finding.clone());
            continue;
        }
        let failures = admission_failures(finding);
        if failures.is_empty() {
            let marked = mark(finding, "assignment_invalid_admitted", Value::Bool(true));
            out.admitted.push(marked.clone());
            out.findings.push(marked);
        } else {
            let downgraded = downgrade(finding, &failures);
            out.rejected.push(serde_json::json!({
                "canonical_task_ids": finding.get("canonical_task_ids"),
                "claim": reason_text(finding),
                "admission_failures": failures,
            }));
            out.findings.push(downgraded);
        }
    }
    out
}

/// The admitted invalid assignments on an already-merged review, or `None` when
/// the review carries none. The driver routes on this.
pub fn escalation(review: &Value) -> Option<Vec<Value>> {
    if review.get("status").and_then(Value::as_str) != Some(VERDICT) {
        return None;
    }
    let admitted = support::array(review.get("assignment_invalid"));
    (!admitted.is_empty()).then_some(admitted)
}

/// Canonical task ids named by the admitted invalid assignments — what the
/// escalation report and any re-scope have to act on.
pub fn invalid_task_ids(admitted: &[Value]) -> Vec<String> {
    support::unique(
        admitted
            .iter()
            .flat_map(|finding| support::strings_of(finding.get("canonical_task_ids")))
            .collect(),
    )
}

fn claims_verdict(finding: &Value) -> bool {
    VERDICT_KEYS
        .iter()
        .filter_map(|key| finding.get(*key).and_then(Value::as_str))
        .any(is_verdict_token)
}

fn is_verdict_token(text: &str) -> bool {
    let normalized = text.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    normalized == VERDICT
}

/// Every admission condition this finding fails, in the order stated in the
/// module comment. Empty means the verdict stands.
fn admission_failures(finding: &Value) -> Vec<String> {
    let mut failures = Vec::new();
    if finding.get("finding_scope").and_then(Value::as_str) == Some("cross_cutting") {
        failures.push(
            "cross-cutting review cannot declare a task's assignment invalid; only the per-task \
             adversarial branch that reviewed it can"
                .to_string(),
        );
    } else if finding.get("attribution_source").and_then(Value::as_str) != Some("review_branch") {
        failures.push(
            "assignment_invalid requires a host-attributed per-task review branch \
             (attribution_source=review_branch)"
                .to_string(),
        );
    }
    if support::strings_of(finding.get("canonical_task_ids")).is_empty() {
        failures.push("assignment_invalid must name the canonical task it invalidates".to_string());
    }
    if reason_text(finding).chars().count() < MIN_REASON_CHARS {
        failures.push(format!(
            "assignment_invalid needs a stated reason of at least {MIN_REASON_CHARS} characters"
        ));
    }
    if evidence_references(finding).is_empty() {
        failures.push(
            "assignment_invalid needs at least one file:line reference showing why the task \
             cannot or should not be done"
                .to_string(),
        );
    }
    failures
}

fn reason_text(finding: &Value) -> String {
    for key in REASON_KEYS {
        if let Some(value) = finding.get(*key).and_then(Value::as_str) {
            let value = value.trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }
    String::new()
}

/// `path:line` references anywhere under the finding's evidence keys.
///
/// The line number is required, not decorative: "src/foo.rs" is a file an agent
/// can name from the task description, whereas "src/foo.rs:212" is a place it
/// had to go and look.
pub fn evidence_references(finding: &Value) -> Vec<String> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(r"[A-Za-z0-9_][A-Za-z0-9_./\\-]*\.[A-Za-z0-9_]{1,12}:\d+")
            .expect("assignment_invalid evidence pattern is a literal")
    });
    let mut found = Vec::new();
    for key in EVIDENCE_KEYS {
        if let Some(value) = finding.get(*key) {
            collect_strings(value, &mut found);
        }
    }
    support::unique(
        found
            .iter()
            .flat_map(|text| {
                pattern
                    .find_iter(text)
                    .map(|hit| hit.as_str().to_string())
                    .collect::<Vec<_>>()
            })
            .collect(),
    )
}

fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(values) => values.iter().for_each(|child| collect_strings(child, out)),
        Value::Object(object) => object
            .values()
            .for_each(|child| collect_strings(child, out)),
        _ => {}
    }
}

/// Strip the verdict and record why it was refused. The finding survives as
/// ordinary remediable work — the round is not lost to a rejected claim.
fn downgrade(finding: &Value, failures: &[String]) -> Value {
    let mut object = finding.as_object().cloned().unwrap_or_default();
    if object.is_empty() {
        return finding.clone();
    }
    for key in VERDICT_KEYS {
        if object
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(is_verdict_token)
        {
            object.remove(*key);
        }
    }
    object.insert(
        "assignment_invalid_rejected".to_string(),
        serde_json::json!(failures),
    );
    Value::Object(object)
}

fn mark(finding: &Value, key: &str, value: Value) -> Value {
    let mut object = finding.as_object().cloned().unwrap_or_default();
    if object.is_empty() {
        return finding.clone();
    }
    object.insert(key.to_string(), value);
    Value::Object(object)
}

#[cfg(test)]
#[path = "assignment_invalid_tests.rs"]
mod tests;
