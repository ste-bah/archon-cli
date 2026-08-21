//! Acceptance-criteria coverage for focused verification plans.
//!
//! The verification planner is handed the whole task universe, which carries
//! every task's authoritative acceptance criteria, and the plan it returns
//! decides what the wave will ever prove. Nothing downstream re-derives that:
//! `verification_inventory_ready` asks only whether the item list is non-empty
//! and issue-free, so a plan promising to check nothing the task was written
//! for is "ready", the wave runs it, and every branch accepts.
//!
//! Observed live on a decomposed PRD: 15 tasks carrying 186 acceptance
//! criteria produced 14 plan items whose entire expected evidence was "cargo
//! check exits 0". Fifty-three branches accepted, zero failed, and the outcome
//! the PRD existed for was never executed once — the run would have reported
//! success having proved only that the code compiles. The no-op path in this
//! same lifecycle has always demanded per-criterion results with evidence
//! refs; this holds verification to the standard no-op already meets.
//!
//! The gate is deliberately about coverage, never content: it asks whether
//! each criterion is claimed by some item, and takes no view on what the
//! criteria say. Nothing here is specific to a PRD, a task, or a domain.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::array;

/// Fields through which a plan item may claim the criteria it covers. The
/// first is what the prompt asks for; the rest are shapes models reach for
/// unprompted, accepted so a plan that did the work is not failed on spelling.
const COVERAGE_FIELDS: [&str; 3] = [
    "covered_acceptance_criteria",
    "acceptance_criteria_coverage",
    "acceptance_criteria",
];

/// Free-text fields that may quote a criterion instead of listing it. Counted
/// as coverage only on a containment match against the full criterion text, so
/// a generic line like "cargo check exits 0" can never absorb a criterion.
const QUOTING_FIELDS: [&str; 2] = ["expected_evidence", "focused_verification"];

/// Acceptance criteria no plan item promised to check, as
/// `{canonical_task_id, criterion}` objects in task then criterion order.
///
/// Empty means every criterion of every candidate task is claimed. Tasks
/// outside `candidate_task_ids` are ignored — a wave verifies the work it
/// implemented, not the whole universe. A candidate carrying no criteria
/// contributes no gaps: absent criteria are the universe parser's problem and
/// are already gated upstream, and failing here would double-report it.
pub fn verification_plan_criteria_gaps(
    task_universe: &Value,
    candidate_task_ids: &[String],
    plan: &Value,
) -> Vec<Value> {
    let candidates: BTreeSet<String> = candidate_task_ids
        .iter()
        .map(|id| normalize(id.as_str()))
        .filter(|id| !id.is_empty())
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    let claimed = claimed_criteria(plan);
    let mut gaps = Vec::new();
    for task in array(task_universe.get("tasks")) {
        let Some(task_id) = task.get("canonical_task_id").and_then(Value::as_str) else {
            continue;
        };
        if !candidates.contains(&normalize(task_id)) {
            continue;
        }
        let covered = claimed.get(&normalize(task_id));
        for criterion in array(task.get("acceptance_criteria")) {
            let Some(text) = criterion.as_str() else {
                continue;
            };
            let normalized = normalize(text);
            if normalized.is_empty() {
                continue;
            }
            if covered.is_some_and(|claims| claims_cover(claims, &normalized)) {
                continue;
            }
            gaps.push(serde_json::json!({
                "canonical_task_id": task_id,
                "criterion": text,
            }));
        }
    }
    gaps
}

/// Every canonical task id named across a set of inventory items, deduplicated
/// and in first-seen order. Callers that hold items rather than a candidate id
/// list use this to ask the gate what those items are answerable for.
pub fn canonical_task_ids_of_items(items: &[Value]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut ids = Vec::new();
    for item in items {
        for id in array(item.get("canonical_task_ids")) {
            let Some(id) = id.as_str() else {
                continue;
            };
            if !id.trim().is_empty() && seen.insert(normalize(id)) {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

/// A criterion is covered when an item claimed it outright, or quoted it whole
/// inside its evidence. Containment runs one way only — the claim must contain
/// the criterion — so broad evidence text cannot swallow a narrow criterion.
fn claims_cover(claims: &BTreeSet<String>, criterion: &str) -> bool {
    claims
        .iter()
        .any(|claim| claim == criterion || claim.contains(criterion))
}

/// Map every canonical task id named by a plan item to the criteria text that
/// item claims. Claims are pooled per task across items: one item may run the
/// command and another inspect the artifact, and between them the task's
/// criteria are covered.
fn claimed_criteria(plan: &Value) -> BTreeMap<String, BTreeSet<String>> {
    let mut claimed: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for item in array(plan.get("items")) {
        let task_ids: Vec<String> = array(item.get("canonical_task_ids"))
            .iter()
            .filter_map(|id| id.as_str().map(normalize))
            .filter(|id| !id.is_empty())
            .collect();
        if task_ids.is_empty() {
            continue;
        }
        let mut claims = BTreeSet::new();
        for field in COVERAGE_FIELDS {
            collect_text(item.get(field), &mut claims);
        }
        for field in QUOTING_FIELDS {
            collect_text(item.get(field), &mut claims);
        }
        if claims.is_empty() {
            continue;
        }
        for task_id in task_ids {
            claimed
                .entry(task_id)
                .or_default()
                .extend(claims.iter().cloned());
        }
    }
    claimed
}

/// Pull criterion text out of whatever shape the field arrived in: a bare
/// string, an array of strings, or an array of objects that name the criterion
/// under one of the keys models actually emit.
fn collect_text(value: Option<&Value>, into: &mut BTreeSet<String>) {
    for entry in array(value) {
        let text = match &entry {
            Value::String(text) => Some(text.clone()),
            Value::Object(fields) => ["criterion", "acceptance_criterion", "text", "summary"]
                .iter()
                .find_map(|key| fields.get(*key).and_then(Value::as_str))
                .map(str::to_string),
            _ => None,
        };
        let Some(text) = text else {
            continue;
        };
        let normalized = normalize(&text);
        if !normalized.is_empty() {
            into.insert(normalized);
        }
    }
}

/// Compare on meaning, not formatting: criteria are copied between a task file,
/// a prompt, and a model reply, and pick up case and whitespace differences on
/// the way that must not read as a missing check.
fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
#[path = "generated_lifecycle_criteria_tests.rs"]
mod tests;
