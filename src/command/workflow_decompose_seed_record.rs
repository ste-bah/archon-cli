use anyhow::Result;
use archon_workflow::{WorkflowV2CallRecord, WorkflowV2Status};
use serde_json::Value;

use super::super::seed_unmapped as unmapped;

const ACCEPTANCE_GATE: &str = "freeze-acceptance";

pub(super) fn acceptance_candidate(record: &WorkflowV2CallRecord) -> Option<&str> {
    let request = record.call.options.host_command.as_ref()?;
    (request.command_id == ACCEPTANCE_GATE
        && record
            .result
            .data
            .get("gateEnvelope")
            .is_some_and(Value::is_object)
        && matches!(
            record.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::NeedsReview | WorkflowV2Status::Noop
        )
        && record.invalidated_by.is_none()
        && record.result.data["gateEnvelope"]
            .get("operational_error")
            .is_none_or(Value::is_null))
    .then(|| request.stdin.as_deref().unwrap_or_default())
}

pub(super) fn authored_criterion(record: &WorkflowV2CallRecord, id: &str) -> Option<String> {
    let task = record.call.options.task.as_deref()?;
    let marker = format!("Author ONLY entry {id}: ");
    let start = task.rfind(&marker)? + marker.len();
    let rest = &task[start..];
    let end = [
        rest.find("\nThe host refused this entry's last answered reply."),
        rest.find("\nAll criterion IDs and text (for consistency):"),
        id.starts_with("SUP-")
            .then(|| rest.find("\nIts covers is exactly [\""))
            .flatten(),
    ]
    .into_iter()
    .flatten()
    .min()
    .unwrap_or(rest.len());
    let authored = rest[..end].trim_end();
    let criterion = if id.starts_with("SUP-") {
        let prefix = authored.find("): ")? + 3;
        &authored[prefix..]
    } else {
        authored
    };
    Some(criterion.to_string())
}

pub(super) use crate::command::workflow_task_set::acceptance_reply_blocks::extract_object;
use crate::command::workflow_task_set::acceptance_reply_blocks::resolve_command_block;

/// The entry `id` a reply holds, as the script's `unwrapEntry` reads it.
pub(super) fn reply_entry(text: &str, id: &str) -> Option<Value> {
    let parsed: Value = serde_json::from_str(text).ok()?;
    if parsed["id"] == id {
        return Some(parsed);
    }
    match parsed["acceptance"].as_array().map(Vec::as_slice) {
        Some([only]) if only["id"] == id => Some(only.clone()),
        _ => None,
    }
}

pub(super) fn reply_entry_with_blocks(content: &str, id: &str) -> Option<Value> {
    let text = extract_object(content);
    let mut entry = reply_entry(&text, id)?;
    resolve_command_block(&mut entry, content)
        .is_none()
        .then_some(entry)
}

/// This build's freeze shape refusals of one carried entry.
pub(super) fn entry_refusals(id: &str, entry: &Value) -> Result<Vec<String>> {
    let candidate = serde_json::to_vec(&serde_json::json!({ "entries": [entry] }))?;
    Ok(crate::command::workflow::element_shape_defects(
        &candidate,
        &crate::command::workflow::ENTRY_SHAPE,
    )
    .iter()
    .map(|defect| {
        format!(
            "acceptance entry '{id}' was refused: {}",
            defect.message.replacen("entries/0", "entry", 1)
        )
    })
    .collect())
}

/// The entries of a recorded acceptance candidate, in its own order.
pub(super) fn candidate_entries(stdin: &str, gate: &str) -> Result<Vec<(String, Value, bool)>> {
    let field = format!("v2/results/{gate}.call.options.host_command.stdin");
    let document: Value =
        serde_json::from_str(stdin).map_err(|e| unmapped(&field, &e.to_string()))?;
    let mut out = Vec::new();
    for (list, supplementary) in [("entries", false), ("supplementary", true)] {
        let Some(items) = document.get(list) else {
            if list == "entries" {
                return Err(unmapped(&field, "the candidate has no entries list"));
            }
            continue;
        };
        for item in items.as_array().into_iter().flatten() {
            let id = item["id"]
                .as_str()
                .ok_or_else(|| unmapped(&field, "a candidate entry has no string id"))?;
            out.push((id.to_string(), item.clone(), supplementary));
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "workflow_decompose_seed_record_tests.rs"]
mod tests;
