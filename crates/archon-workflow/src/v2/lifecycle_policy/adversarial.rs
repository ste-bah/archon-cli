// Per-task adversarial review — the map half of the review diamond.
//
// WHY THIS IS NOT A REDUCE ANY MORE
//
// Adversarial review used to be ONE terminal `reduce` over every task at once.
// Three properties followed from that shape, and all three were observed live
// on run `wf-ee4a92fc` (17 tasks):
//
//  1. Attribution was inferential. A reducer holding every task must GUESS
//     which task each finding belongs to, and it forgot: all 43 findings came
//     back with no task key of any kind, so `findingsByTask` routed 100% of
//     them to `unassigned` and `remediateFindings` returned them untouched.
//     The stamping fix in `workflow_live_v3_primitives.js` recovers attribution
//     from the emitting branch — but a reduce has no per-item branch, so that
//     fix could not apply to this stage at all.
//  2. Findings arrived after every wave, when a wave-1 defect already has
//     downstream work built on top of it.
//  3. One context had to hold 17 tasks' worth of diffs.
//
// Reviewing one task per branch makes attribution STRUCTURAL: the branch a
// finding came out of names exactly one task, and the branch id is stamped by
// the host (`WorkflowV2BranchOutcome::item_id`), never by the model. A reviewer
// that emits a finding with no task key at all is still attributed correctly,
// because nothing was ever asked of it. That is the same argument the primitives
// carry: "a field the model is asked to remember is a field it can forget — and
// when it forgets, the finding is silently dropped from remediation rather than
// erroring."
//
// The terminal reduce survives, narrowed and renamed to `cross-cutting-review`:
// contradictions BETWEEN tasks, global invariants, PRD-level acceptance. It is
// handed a compact DIGEST of the per-task findings rather than the run's
// implementation and verification evidence, so it cannot re-review per-task
// work; `merge_review` then drops any cross-cutting item that duplicates a
// per-task finding identity, so duplication is prevented structurally rather
// than by instruction.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::generated_lifecycle_support as support;
use crate::task_universe::WorkflowV2TaskUniverse;

/// Call-id prefix for the per-task review stage. One `parallel` call per
/// dependency wave / review round; one BRANCH per task inside it.
pub(crate) const PER_TASK_REVIEW_ITEM_PREFIX: &str = "adversarial-review-";

/// Keys a finding may use to name its own tasks. Read, never required.
const DECLARED_TASK_ID_KEYS: &[&str] = &["canonical_task_ids", "task_ids", "taskIds", "task_id"];

/// Where a reviewer may put its findings inside a branch result.
const FINDING_COLLECTION_KEYS: &[&str] = &["findings", "review_findings", "items", "issues"];

/// One review item per task: this task's acceptance criteria, this task's
/// declared `## Adversarial Review Notes`, and this task's own evidence.
///
/// `item_id` is derived from the canonical task id, so the host-stamped branch
/// id is a total function of the task under review — that identity is what
/// makes attribution structural in `attributed_findings`.
pub fn per_task_review_items(
    universe: &WorkflowV2TaskUniverse,
    task_ids: &BTreeSet<String>,
    evidence: &[&[Value]],
) -> Vec<Value> {
    universe
        .tasks
        .iter()
        .filter(|task| task_ids.contains(&task.canonical_task_id))
        .map(|task| {
            let id = task.canonical_task_id.as_str();
            serde_json::json!({
                "item_id": review_item_id(id),
                "canonical_task_ids": [id],
                "task_id": id,
                "task_file": task.source_path,
                "task_title": task.title,
                "acceptance_criteria": task.acceptance_criteria,
                "adversarial_review_notes": task.adversarial_review_notes,
                "artifact_requirements": task.artifact_requirements,
                "review_scope": "single_task",
                "task_evidence": task_scoped_evidence(evidence, id),
            })
        })
        .collect()
}

pub fn review_item_id(canonical_task_id: &str) -> String {
    format!("{PER_TASK_REVIEW_ITEM_PREFIX}{canonical_task_id}")
}

/// Only the evidence entries that name this task. A reviewer that cannot see a
/// sibling task's diff cannot be confused by it, and cannot spend its context
/// on it.
fn task_scoped_evidence(bundles: &[&[Value]], task_id: &str) -> Vec<Value> {
    let mut scoped = Vec::new();
    for bundle in bundles {
        for entry in bundle.iter() {
            if value_names_task(entry, task_id) {
                scoped.push(entry.clone());
            }
        }
    }
    scoped
}

fn value_names_task(value: &Value, task_id: &str) -> bool {
    match value {
        Value::String(text) => text == task_id,
        Value::Array(values) => values.iter().any(|value| value_names_task(value, task_id)),
        Value::Object(object) => object
            .iter()
            .any(|(_, value)| value_names_task(value, task_id)),
        _ => false,
    }
}

/// Attribute every finding to the task whose BRANCH produced it.
///
/// The branch id is host-stamped from the item this module built, so the
/// `item_id -> canonical task id` map is authoritative. Findings keep any ids
/// they declared themselves (a reviewer may legitimately cross-reference), but
/// the branch's own task id is ALWAYS present — that is the property that makes
/// a finding with no task key at all impossible to lose.
///
/// Fail-closed fallback: when a branch outcome carries no usable `item_id` and
/// the outcome count matches the item count, position is used. When neither
/// holds, the finding is returned UNSTAMPED rather than guessed at, so it
/// surfaces as unassigned instead of being silently attached to the wrong task.
pub fn attributed_findings(items: &[Value], envelope: &Value) -> Vec<Value> {
    let by_item = task_ids_by_item_id(items);
    let outcomes = support::outcomes_of(envelope);
    let positional = outcomes.len() == items.len();
    let mut collected = Vec::new();
    for (index, outcome) in outcomes.iter().enumerate() {
        let branch_task = outcome
            .get("item_id")
            .or_else(|| outcome.get("id"))
            .and_then(Value::as_str)
            .and_then(|item_id| by_item.get(item_id).cloned())
            .or_else(|| {
                if positional {
                    items
                        .get(index)
                        .and_then(|item| item.get("task_id"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                } else {
                    None
                }
            });
        for finding in findings_of_outcome(outcome) {
            collected.push(stamp_task_id(finding, branch_task.as_deref()));
        }
    }
    collected
}

fn task_ids_by_item_id(items: &[Value]) -> BTreeMap<String, String> {
    items
        .iter()
        .filter_map(|item| {
            let item_id = item.get("item_id")?.as_str()?.to_string();
            let task_id = item.get("task_id")?.as_str()?.to_string();
            Some((item_id, task_id))
        })
        .collect()
}

fn findings_of_outcome(outcome: &Value) -> Vec<Value> {
    let mut roots = vec![outcome];
    if let Some(result) = outcome.get("result") {
        roots.push(result);
        if let Some(data) = result.get("data") {
            roots.push(data);
        }
    }
    if let Some(data) = outcome.get("data") {
        roots.push(data);
    }
    for root in roots {
        for key in FINDING_COLLECTION_KEYS {
            let values = support::array(root.get(*key));
            if !values.is_empty() {
                return values;
            }
        }
    }
    Vec::new()
}

/// Union the branch's task id into the finding's declared ids. Never removes an
/// id the reviewer supplied; never omits the branch's own.
fn stamp_task_id(finding: Value, branch_task: Option<&str>) -> Value {
    let Some(branch_task) = branch_task else {
        return finding;
    };
    let mut declared = declared_task_ids(&finding);
    if !declared.iter().any(|id| id == branch_task) {
        declared.insert(0, branch_task.to_string());
    }
    let mut object = finding.as_object().cloned().unwrap_or_else(|| {
        let mut object = serde_json::Map::new();
        object.insert("finding".to_string(), finding.clone());
        object
    });
    object.insert(
        "canonical_task_ids".to_string(),
        serde_json::json!(declared),
    );
    object.insert(
        "attribution_source".to_string(),
        Value::String("review_branch".to_string()),
    );
    Value::Object(object)
}

fn declared_task_ids(finding: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    for key in DECLARED_TASK_ID_KEYS {
        match finding.get(*key) {
            Some(Value::String(value)) if !value.trim().is_empty() => {
                ids.push(value.trim().to_string());
            }
            Some(value @ Value::Array(_)) => ids.extend(support::strings_of(Some(value))),
            _ => {}
        }
    }
    support::unique(ids)
}

/// Every per-task finding this run has recorded, newest last. Read back out of
/// `evidence.review` so the terminal reduce round does not have to re-run the
/// per-task stage to know what the wave-time reviewers already found.
pub fn collected_per_task_findings(review_evidence: &[Value]) -> Vec<Value> {
    review_evidence
        .iter()
        .filter(|entry| {
            entry.get("kind").and_then(Value::as_str) == Some("adversarial-review-task")
        })
        .flat_map(|entry| support::array(entry.get("findings")))
        .collect()
}

#[cfg(test)]
#[path = "adversarial_tests.rs"]
mod tests;

/// Stable identity of a finding, used to prove the terminal reduce is not
/// re-emitting per-task work. Mirrors `findingIdentities` in
/// `workflow_live_v3_primitives.js` so both halves of the system agree on what
/// "the same finding" means.
pub(crate) fn finding_identities(finding: &Value) -> Vec<String> {
    let mut keys = Vec::new();
    for key in [
        "id",
        "title",
        "claim",
        "summary",
        "finding",
        "requirement_id",
    ] {
        if let Some(value) = finding.get(key).and_then(Value::as_str) {
            let value = value.trim();
            if !value.is_empty() {
                keys.push(format!(
                    "{key}:{}",
                    value.chars().take(200).collect::<String>()
                ));
            }
        }
    }
    keys
}
