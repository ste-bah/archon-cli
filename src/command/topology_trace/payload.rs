//! Reading typed values out of untyped payloads.
//!
//! Both taps have to mine a `serde_json::Value` someone else defined — a tool's
//! input object, a workflow event's `detail` — for the few facts the trace
//! needs. Every key name that is guessed at rather than declared lives here, so
//! that guessing is in one auditable place.
//!
//! Nothing here records tool input verbatim: only extracted paths and names
//! leave this module, which is what keeps the trace from becoming a secret sink.

use archon_topology::ir::WriteTarget;

/// Tool input keys that name a file the tool writes.
const WRITE_PATH_KEYS: &[&str] = &["file_path", "path", "notebook_path", "target_file"];

/// Tools that write files. Restricting extraction to these avoids recording a
/// `Read`'s `file_path` as a write.
const WRITING_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "ApplyPatch", "NotebookEdit"];

/// Files a tool call wrote, read out of its input.
///
/// Only for tools known to write. A `Read` also carries `file_path`, and
/// recording that as a write would manufacture write conflicts out of nothing.
pub(crate) fn written_paths(tool_name: &str, input: &serde_json::Value) -> Vec<WriteTarget> {
    if !WRITING_TOOLS.contains(&tool_name) {
        return Vec::new();
    }
    let Some(fields) = input.as_object() else {
        return Vec::new();
    };
    let mut targets: Vec<WriteTarget> = WRITE_PATH_KEYS
        .iter()
        .filter_map(|key| fields.get(*key))
        .filter_map(serde_json::Value::as_str)
        .filter(|path| !path.is_empty())
        .map(|path| WriteTarget::Path(normalize_write_path(path)))
        .collect();
    targets.sort();
    targets.dedup();
    targets
}

/// Write targets are compared by exact string
/// (`TaskGraph::write_conflicts`), so separators must agree or two writes to
/// the same file look unrelated. Absolute prefixes are left alone: stripping
/// them needs a project root this function does not have, and an over-long key
/// under-reports conflicts rather than inventing them.
fn normalize_write_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// The agent type a subagent-spawning tool call named, if any.
pub(crate) fn subagent_type(input: &serde_json::Value) -> Option<String> {
    let fields = input.as_object()?;
    ["subagent_type", "agent_type", "agent", "type"]
        .iter()
        .filter_map(|key| fields.get(*key))
        .filter_map(serde_json::Value::as_str)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

/// Stage identifier out of a workflow event's untyped `detail` payload.
pub(super) fn workflow_stage_id(detail: &serde_json::Value) -> Option<String> {
    let fields = detail.as_object()?;
    ["stage", "stage_id", "id", "name"]
        .iter()
        .filter_map(|key| fields.get(*key))
        .filter_map(serde_json::Value::as_str)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

/// Declared write targets out of a workflow event's `detail` payload.
pub(super) fn workflow_stage_writes(detail: &serde_json::Value) -> Option<Vec<WriteTarget>> {
    let fields = detail.as_object()?;
    let values = ["target_files", "expected_target_files", "writes"]
        .iter()
        .find_map(|key| fields.get(*key))
        .and_then(serde_json::Value::as_array)?;
    let mut targets: Vec<WriteTarget> = values
        .iter()
        .filter_map(serde_json::Value::as_str)
        .filter(|path| !path.is_empty())
        .map(|path| WriteTarget::Path(normalize_write_path(path)))
        .collect();
    if targets.is_empty() {
        return None;
    }
    targets.sort();
    targets.dedup();
    Some(targets)
}
