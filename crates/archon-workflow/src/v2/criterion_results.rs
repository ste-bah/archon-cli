//! Batch O (Issue-213 C6b): a write call answers for EACH acceptance
//! criterion of the tasks it claims.
//!
//! A write agent used to be asked to do a task and to judge its own
//! completion with nothing per-criterion in the answer, so "accepted" could
//! stand for a task half of whose criteria were never looked at. Now an
//! implementation or remediation write call reports, under
//! [`CRITERION_RESULTS_KEY`] (the agent adapter lifts a top-level field of
//! that name into `data`, as it does `finding_dispositions`), one entry per
//! criterion of every task it claims:
//!
//! `{task_id, criterion_index, criterion, status: "met" | "unmet", evidence}`
//!
//! The criteria come from the host's task universe (the same scoped copy the
//! request carries), never from the agent. A branch that claims `accepted`
//! while any criterion has no entry, is not `met`, or has no evidence is not
//! accepted: it is demoted to `needs_review`, every such criterion named in
//! a residual gap, and its work is kept as partial work for the next attempt.
//! A schema checks shape, not truth: this bounds what "accepted" can claim;
//! the verifiers still judge whether the claim holds.

use serde_json::Value;

use crate::v2::{
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2ResidualGap, WorkflowV2Result,
    WorkflowV2Status,
};

/// Key of the per-criterion results in a write result's `data`.
pub const CRITERION_RESULTS_KEY: &str = "criterion_results";
/// Key of the host's reading of them, stamped on the result.
pub const CRITERION_CHECK_KEY: &str = "criterion_check";
/// Prefix of the residual gap naming a task's unmet criteria.
pub const UNMET_CRITERIA_GAP_PREFIX: &str = "unmet_acceptance_criteria_";

/// One acceptance criterion a call answers for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Criterion {
    pub task_id: String,
    /// 1-based, in the order the task declares them.
    pub index: usize,
    pub text: String,
}

/// The criteria of every task the call's input claims, from the task
/// universe the host put on that input. Empty when the input claims nothing,
/// carries no universe, or its tasks declare no criteria.
pub fn claimed_criteria(input: &Value) -> Vec<Criterion> {
    let claimed = crate::v2::branch_stamping::branch_canonical_task_ids(input);
    if claimed.is_empty() {
        return Vec::new();
    }
    let Some(tasks) = find_universe(input)
        .and_then(|universe| universe.get("tasks"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut criteria = Vec::new();
    for id in &claimed {
        let Some(task) = tasks.iter().find(|task| task_named(task, id)) else {
            continue;
        };
        let canonical = task
            .get("canonical_task_id")
            .and_then(Value::as_str)
            .unwrap_or(id);
        if criteria.iter().any(|c: &Criterion| c.task_id == canonical) {
            continue;
        }
        let declared = task.get("acceptance_criteria").and_then(Value::as_array);
        for (n, text) in declared
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .enumerate()
        {
            criteria.push(Criterion {
                task_id: canonical.to_string(),
                index: n + 1,
                text: text.trim().to_string(),
            });
        }
    }
    criteria
}

fn task_named(task: &Value, id: &str) -> bool {
    task.get("canonical_task_id").and_then(Value::as_str) == Some(id)
        || task
            .get("aliases")
            .and_then(Value::as_array)
            .is_some_and(|aliases| aliases.iter().any(|alias| alias.as_str() == Some(id)))
}

fn find_universe(value: &Value) -> Option<&Value> {
    match value {
        Value::Object(object) => ["task_universe", "taskUniverse"]
            .iter()
            .find_map(|key| object.get(*key).filter(|u| u.get("tasks").is_some()))
            .or_else(|| object.values().find_map(find_universe)),
        Value::Array(values) => values.iter().find_map(find_universe),
        _ => None,
    }
}

/// Why `criterion` is not met by `data`'s entries, or `None` when it is.
fn unmet_reason(criterion: &Criterion, single_task: bool, data: &Value) -> Option<String> {
    let entries = data.get(CRITERION_RESULTS_KEY).and_then(Value::as_array);
    let entry = entries.into_iter().flatten().rev().find(|entry| {
        let task = entry.get("task_id").and_then(Value::as_str).map(str::trim);
        let task_ok = task.map_or(single_task, |task| task == criterion.task_id);
        let index = entry
            .get("criterion_index")
            .or_else(|| entry.get("index"))
            .and_then(Value::as_u64);
        let text = entry
            .get("criterion")
            .and_then(Value::as_str)
            .map(str::trim);
        task_ok
            && (index == Some(criterion.index as u64)
                || (index.is_none() && text == Some(criterion.text.as_str())))
    });
    let Some(entry) = entry else {
        return Some("no criterion_results entry names it".to_string());
    };
    let status = entry
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if status != "met" {
        return Some(if status.is_empty() {
            "its entry gives no status".to_string()
        } else {
            format!("its entry says `{status}`")
        });
    }
    let evidenced = match entry.get("evidence") {
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(object)) => !object.is_empty(),
        _ => false,
    };
    (!evidenced).then(|| "its entry gives no evidence".to_string())
}

/// Hold an accepted write result to its per-criterion contract; see the
/// module doc. A result that is not `accepted`, or a call with no criteria to
/// answer for, is left as it is.
pub fn enforce(input: &Value, result: &mut WorkflowV2Result) {
    if result.status != WorkflowV2Status::Accepted {
        return;
    }
    let criteria = claimed_criteria(input);
    if criteria.is_empty() {
        return;
    }
    let single_task = criteria.iter().all(|c| c.task_id == criteria[0].task_id);
    let unmet: Vec<(&Criterion, String)> = criteria
        .iter()
        .filter_map(|c| unmet_reason(c, single_task, &result.data).map(|why| (c, why)))
        .collect();
    let check = serde_json::json!({
        "required": criteria.len(),
        "met": criteria.len() - unmet.len(),
        "unmet": unmet.iter().map(|(c, why)| serde_json::json!({
            "task_id": c.task_id,
            "criterion_index": c.index,
            "criterion": c.text,
            "reason": why,
        })).collect::<Vec<_>>(),
    });
    if !result.data.is_object() {
        result.data = serde_json::json!({});
    }
    if let Some(data) = result.data.as_object_mut() {
        data.insert(CRITERION_CHECK_KEY.to_string(), check);
    }
    if unmet.is_empty() {
        return;
    }
    result.status = WorkflowV2Status::NeedsReview;
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        format!(
            "host demoted an accepted write result: {} of {} acceptance criteria were not reported met with evidence",
            unmet.len(),
            criteria.len()
        ),
    ));
    let mut tasks: Vec<&str> = unmet.iter().map(|(c, _)| c.task_id.as_str()).collect();
    tasks.dedup();
    for task in tasks {
        let named = unmet
            .iter()
            .filter(|(c, _)| c.task_id == task)
            .map(|(c, why)| format!("#{} \"{}\" ({why})", c.index, c.text))
            .collect::<Vec<_>>()
            .join("; ");
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: format!("{UNMET_CRITERIA_GAP_PREFIX}{}", sanitize(task)),
            description: format!(
                "task {task} was claimed accepted without every acceptance criterion reported met with evidence: {named}"
            ),
            severity: Some("review".to_string()),
        });
    }
}

fn sanitize(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

/// The prompt section asking a write call for its per-criterion results;
/// empty when the call has no criteria to answer for.
pub fn prompt_section(input: &Value) -> String {
    let criteria = claimed_criteria(input);
    if criteria.is_empty() {
        return String::new();
    }
    let mut text = String::from(
        "## Acceptance Criteria Results (required)\n\
         Return data.criterion_results: one entry for EVERY criterion below, \
         {\"task_id\", \"criterion_index\", \"criterion\", \"status\": \"met\" | \"unmet\", \
         \"evidence\": what you ran or inspected on this tree that establishes it (quote the output)}. \
         An `accepted` result with any criterion missing, not `met`, or without evidence is not \
         accepted: the host returns it as needs_review naming those criteria. If a criterion is \
         not met, say so honestly.\n",
    );
    for c in &criteria {
        text.push_str(&format!("- {} #{}: {}\n", c.task_id, c.index, c.text));
    }
    text.push('\n');
    text
}

#[cfg(test)]
#[path = "criterion_results_tests.rs"]
mod tests;
