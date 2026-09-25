//! Which task a review finding belongs to, in the one shape every reader
//! uses: `canonical_task_ids`, non-empty whenever any spelling names a task.
//!
//! The prelude routes a finding by the FIRST present spelling (`canonical_task_ids
//! || task_ids || ...`), so `{canonical_task_ids: [], task_id: "T"}` routed
//! nowhere while [`task_ids_of`] read it as T. The host now normalises every
//! finding it attaches, so both read the same list.

use serde_json::{Map, Value};

/// The task ids a value declares, under any of the spellings agents use.
pub fn task_ids_of(value: &Value) -> Vec<String> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    for key in ["canonical_task_ids", "task_ids", "taskIds", "task_id"] {
        let ids = match object.get(key) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>(),
            Some(Value::String(id)) if !id.trim().is_empty() => vec![id.trim().to_string()],
            _ => Vec::new(),
        };
        if !ids.is_empty() {
            return ids;
        }
    }
    Vec::new()
}

/// Stamp `canonical_task_ids` onto a finding that declares none. Attribution a
/// reviewer supplied itself is never overwritten -- a finding that legitimately
/// names several tasks keeps all of them -- and a non-object finding (a bare
/// requirement id, say) is returned untouched rather than shredded.
pub fn stamp_task_ids(finding: Value, task_ids: &[String]) -> Value {
    let Value::Object(mut object) = finding else {
        return finding;
    };
    if task_ids.is_empty() || !task_ids_of(&Value::Object(object.clone())).is_empty() {
        return Value::Object(object);
    }
    object.insert(
        "canonical_task_ids".to_string(),
        Value::from(task_ids.to_vec()),
    );
    Value::Object(object)
}

/// A bare-string finding as an object carrying its text under `claim`, so it
/// can hold the task of the branch that produced it. The text is preserved.
pub fn wrap_bare(finding: Value) -> Value {
    match finding {
        Value::String(text) => {
            let mut object = Map::new();
            object.insert("claim".to_string(), Value::String(text));
            Value::Object(object)
        }
        other => other,
    }
}

/// Rewrite `canonical_task_ids` to the ids [`task_ids_of`] reads (trimmed,
/// de-duplicated, first non-empty spelling). A finding naming no task is
/// returned unchanged.
pub fn normalize_task_ids(finding: Value) -> Value {
    let ids = task_ids_of(&finding);
    let Value::Object(mut object) = finding else {
        return finding;
    };
    if ids.is_empty() {
        return Value::Object(object);
    }
    let mut unique = Vec::new();
    for id in ids {
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    object.insert("canonical_task_ids".to_string(), Value::from(unique));
    Value::Object(object)
}
