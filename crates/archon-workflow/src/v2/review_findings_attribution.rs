//! Which task a review finding belongs to, in the one shape every reader
//! uses: `canonical_task_ids`, non-empty whenever any spelling names a task.
//!
//! The prelude routes a finding by the FIRST present spelling (`canonical_task_ids
//! || task_ids || ...`), so `{canonical_task_ids: [], task_id: "T"}` routed
//! nowhere while [`task_ids_of`] read it as T. The host now normalises every
//! finding it attaches, so both read the same list.

use serde_json::{Map, Value};

use crate::task_universe::WorkflowV2TaskUniverse;

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

/// [`normalize_task_ids_in`] without a task universe: trimmed and
/// de-duplicated, spellings kept.
pub fn normalize_task_ids(finding: Value) -> Value {
    normalize_task_ids_in(finding, None)
}

/// Rewrite a finding's attribution into the one list every reader uses:
/// `canonical_task_ids` holds the ids [`task_ids_of`] reads (first non-empty
/// spelling, trimmed, de-duplicated).
///
/// With the task universe, each id is resolved to the universe's own
/// spelling (case-insensitively, through canonical ids and aliases); an id the
/// universe does not know (a requirement id, a stray alias) moves to
/// `referenced_ids`, and the other spellings are removed, so the prelude's
/// router and the host's terminal rule see exactly the same task list. A
/// finding naming no task is returned unchanged.
pub fn normalize_task_ids_in(finding: Value, universe: Option<&WorkflowV2TaskUniverse>) -> Value {
    let ids = task_ids_of(&finding);
    let Value::Object(mut object) = finding else {
        return finding;
    };
    if ids.is_empty() {
        return Value::Object(object);
    }
    let mut tasks: Vec<String> = Vec::new();
    let mut referenced: Vec<String> = Vec::new();
    for id in ids {
        let (bucket, id) = match universe {
            None => (&mut tasks, id),
            Some(universe) => match resolve(universe, &id) {
                Some(task) => (&mut tasks, task),
                None => (&mut referenced, id),
            },
        };
        if !bucket.contains(&id) {
            bucket.push(id);
        }
    }
    if universe.is_some() {
        for key in ["task_ids", "taskIds", "task_id"] {
            object.remove(key);
        }
        if !referenced.is_empty() {
            let mut all: Vec<Value> = object
                .get(REFERENCED_IDS_KEY)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            all.extend(referenced.into_iter().map(Value::from));
            object.insert(REFERENCED_IDS_KEY.to_string(), Value::Array(all));
        }
    }
    object.insert("canonical_task_ids".to_string(), Value::from(tasks));
    Value::Object(object)
}

/// Where a finding's ids that name no universe task are kept.
pub const REFERENCED_IDS_KEY: &str = "referenced_ids";

/// The universe's spelling of `id`: a canonical id or an alias, compared
/// case-insensitively.
fn resolve(universe: &WorkflowV2TaskUniverse, id: &str) -> Option<String> {
    let id = id.trim();
    universe
        .tasks
        .iter()
        .find(|task| {
            task.canonical_task_id.eq_ignore_ascii_case(id)
                || task
                    .aliases
                    .iter()
                    .any(|alias| alias.trim().eq_ignore_ascii_case(id))
        })
        .map(|task| task.canonical_task_id.clone())
}
