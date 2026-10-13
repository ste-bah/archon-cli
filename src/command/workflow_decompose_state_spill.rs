//! Progress events past their cap (Issue 386, item 4).
//!
//! The cap bounds one line of `events.jsonl`; it is not a reason to fail the
//! run. A larger event is written whole, sanitized exactly as the logged
//! event is, to `decomposition/progress-event-spills/event-<seq>.json`, and
//! the log carries a bounded view instead: every field whose own encoding is
//! short, the digest of each field that is not, and the spill's path and
//! size. The operator log and the UI read the full detail in memory.

use archon_workflow::{WorkflowError, WorkflowResult, WorkflowStore};
use serde_json::{Map, Value};

pub(crate) const PROGRESS_EVENT_SPILLS_DIR: &str = "decomposition/progress-event-spills";

/// The longest encoded field a view keeps as it is.
const VIEW_FIELD_BYTES: usize = 512;

/// `detail` as `events.jsonl` logs it: unchanged within `limit` bytes,
/// otherwise spilled whole and replaced by its bounded view.
pub(super) fn bounded_event(
    store: &WorkflowStore,
    run_id: &str,
    seq: u64,
    detail: &Value,
    limit: usize,
) -> WorkflowResult<Value> {
    let encoded = serde_json::to_vec(detail)?;
    if encoded.len() <= limit {
        return Ok(detail.clone());
    }
    let relative = format!("{PROGRESS_EVENT_SPILLS_DIR}/event-{seq}.json");
    store.write_run_file(run_id, &relative, &encoded)?;
    let mut view = Map::new();
    if let Value::Object(fields) = detail {
        for (key, value) in fields {
            let bytes = serde_json::to_vec(value)?;
            if bytes.len() <= VIEW_FIELD_BYTES {
                view.insert(key.clone(), value.clone());
            } else {
                view.insert(
                    format!("{key}_digest"),
                    Value::String(archon_workflow::task_set_contract::content_digest(&bytes)),
                );
            }
        }
    }
    view.insert("detail_truncated".into(), Value::Bool(true));
    view.insert("detail_bytes".into(), Value::from(encoded.len() as u64));
    view.insert("full_detail".into(), Value::String(relative.clone()));
    let view = Value::Object(view);
    let view_bytes = serde_json::to_vec(&view)?.len();
    if view_bytes > limit {
        // Only a detail with a great many short fields reaches this; the
        // fixed projection has ten. The full event is on disk already.
        return Err(WorkflowError::StageFailed(format!(
            "fixed decomposition progress event {seq} was spilled to {relative}, but its bounded view is still {view_bytes} bytes, over {limit}"
        )));
    }
    Ok(view)
}

#[cfg(test)]
#[path = "workflow_decompose_state_spill_tests.rs"]
mod tests;
