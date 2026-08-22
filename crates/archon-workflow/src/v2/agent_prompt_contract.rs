//! Task-universe digests, and the contract context a call is shown.
//!
//! Split from `agent_prompt` so neither file carries the whole prompt
//! assembly plus the universe reduction it depends on.

/// Whether a universe task is one of the ids this call claims.
fn task_is_claimed(task: &serde_json::Value, claimed: &[String]) -> bool {
    let mut names = Vec::new();
    if let Some(id) = task.get("canonical_task_id").and_then(|v| v.as_str()) {
        names.push(id);
    }
    names.extend(
        task.get("aliases")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str),
    );
    names.iter().any(|name| claimed.iter().any(|id| id == name))
}

/// Attach the declared contracts this call is answerable for.
///
/// Scoped to the ids the call claims, because the whole universe is the wrong
/// answer for a write agent: one reference decomposition carries 186 acceptance
/// criteria across fifteen tasks, and pasting all of them into an agent asked to
/// satisfy sixteen of them buries its own contract in fourteen other tasks'.
///
/// A call claiming nothing — a reducer surveying the run, a final report — still
/// gets everything, because that is genuinely its subject. And a claim that
/// resolves to no task falls back to everything rather than to silence: an
/// unresolvable id is a defect worth surfacing elsewhere, but starving the agent
/// of its contract is the failure this whole path exists to prevent.
pub(super) fn insert_task_contract_context(
    invocation: &mut serde_json::Value,
    universes: &[serde_json::Value],
    claimed_task_ids: &[String],
) {
    let all = || {
        universes.iter().flat_map(|universe| {
            universe
                .get("tasks")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
        })
    };
    let mut tasks = all()
        .filter(|task| claimed_task_ids.is_empty() || task_is_claimed(task, claimed_task_ids))
        .map(task_contract_digest)
        .collect::<Vec<_>>();
    if tasks.is_empty() {
        tasks = all().map(task_contract_digest).collect();
    }
    let context = serde_json::json!({"tasks":tasks});
    match invocation {
        serde_json::Value::Object(object) => {
            object.insert("task_contract_context".to_string(), context);
        }
        serde_json::Value::Array(values) => {
            values.push(serde_json::json!({"task_contract_context":context}));
        }
        _ => {}
    }
}

pub(super) fn task_universe_digest(universe: &serde_json::Value) -> serde_json::Value {
    let Some(object) = universe.as_object() else {
        return universe.clone();
    };
    let digests = object
        .get("tasks")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|task| serde_json::Value::Object(task_digest_fields(task)))
        .collect::<Vec<_>>();
    serde_json::json!({
        "schema_version": object.get("schema_version"),
        "source_roots": object.get("source_roots"),
        "tasks": digests,
    })
}

fn task_contract_digest(task: &serde_json::Value) -> serde_json::Value {
    let mut digest = task_digest_fields(task);
    for key in ["acceptance_criteria", "deliverable_contracts"] {
        if let Some(value) = task.get(key) {
            digest.insert(key.to_string(), value.clone());
        }
    }
    serde_json::Value::Object(digest)
}

fn task_digest_fields(task: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    let mut digest = serde_json::Map::new();
    for key in [
        "canonical_task_id",
        "aliases",
        "source_path",
        "dependency_ids",
        "artifact_requirements",
        "required_env_keys",
        "required_tools",
    ] {
        if let Some(value) = task.get(key) {
            digest.insert(key.to_string(), value.clone());
        }
    }
    digest
}
