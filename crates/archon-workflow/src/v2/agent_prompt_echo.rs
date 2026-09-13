//! Rendering-time removal of prompt text the `## Task` section already
//! carries, so the first user message says a branch's prompt once.

use serde_json::Value;

/// Rendering only: retain source inputs for cache identity and execution.
///
/// A branch's prompt reaches the request several ways at once: as the call
/// task (rendered under `## Task`, where the host wraps it in preambles — time
/// budget, resumed partial, repository audit, landing hints), as `item.task`,
/// as `item.instructions` (the primitives emitted both), and on a verifier as
/// `item.verification_requirements[0]`. Measured on one live remediation
/// branch: the same 145,295-char prompt three times in the first user message.
/// Exact equality with the whole `## Task` text was the only case handled, so
/// the moment a preamble was prepended every copy survived. A copy is now
/// removed from the RENDERED input when the `## Task` section already carries
/// it verbatim (equal, or one of its line-bounded segments), and
/// `instructions` is removed when it repeats a retained `task`.
pub(super) fn strip_task_echoes(value: &mut Value, task: &str) {
    if task.is_empty() { return; }
    match value {
        Value::Object(object) => {
            let echoed = |value: &Value| {
                value.as_str().is_some_and(|text| rendered_task_carries(task, text))
            };
            if object.get("task").is_some_and(echoed) {
                object.remove("task");
            }
            if let Some(instructions) = object.get("instructions")
                && (echoed(instructions) || object.get("task") == Some(instructions))
            {
                object.remove("instructions");
            }
            if let Some(Value::Array(entries)) = object.get_mut("verification_requirements") {
                for entry in entries.iter_mut().filter(|entry| echoed(entry)) {
                    *entry = Value::String(VERIFICATION_REQUIREMENT_ECHO.to_string());
                }
            }
            // Only invocation wrappers, never evidence or task-universe records.
            for key in ["options", "inputs", "input", "source_data", "item"] {
                if let Some(nested) = object.get_mut(key) { strip_task_echoes(nested, task); }
            }
        }
        Value::Array(values) => {
            for value in values { strip_task_echoes(value, task); }
        }
        _ => {}
    }
}

/// What a verifier's echoed requirement is replaced with, so the field still
/// says where its text is rather than reading as "no requirement declared".
const VERIFICATION_REQUIREMENT_ECHO: &str = "(identical to the ## Task section above)";

/// Whether the rendered `## Task` text already carries `text` verbatim: equal
/// to it, or one of its newline-bounded segments. Host preambles and suffixes
/// join onto a branch prompt with newlines, so the prompt stays a whole
/// segment; a substring inside a line is not an echo.
fn rendered_task_carries(task: &str, text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    if task == text {
        return true;
    }
    let bytes = task.as_bytes();
    task.match_indices(text).any(|(start, _)| {
        let end = start + text.len();
        (start == 0 || bytes[start - 1] == b'\n') && (end == bytes.len() || bytes[end] == b'\n')
    })
}
