use std::collections::{BTreeSet, VecDeque};

use serde_json::{Value, json};

use crate::error::{WorkflowError, WorkflowResult};
use crate::run::{ArtifactRef, StageStatus, WorkflowRun};
use crate::spec::{StageKind, StageSpec};
use crate::store::WorkflowStore;

pub(crate) fn repair_empty_inventory_output(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
    body: &str,
) -> WorkflowResult<Option<String>> {
    if !crate::spec::stage_declares_items_producer(stage)
        || !crate::stage::is_recovery_stage(stage)
        || !emits_empty_items(body)
    {
        return Ok(None);
    }
    let deps = dependency_closure(run, stage);
    let forced = forced_acceptances(run, &deps);
    if forced.is_empty() {
        return Ok(None);
    }
    let items = normalize_items(remediation_items_from_dependencies(store, run, &deps)?);
    let items = dedupe_items(items);
    if items.is_empty() {
        let (stage_id, reason) = &forced[0];
        return Err(WorkflowError::StageFailed(format!(
            "empty remediation inventory '{}' hid unresolved forced-accepted upstream stage '{}': {}; no parseable remediation item/finding evidence was available",
            stage.id, stage_id, reason
        )));
    }
    if feeds_implementation_fanout(run, stage) {
        ensure_items_have_targets(stage, &items)?;
    }
    serde_json::to_string_pretty(&json!({
        "schema": "archon.workflow.remediation_inventory.v1",
        "source": "workflow_synthesized_from_upstream_failure_evidence",
        "inventory_stage": stage.id,
        "forced_accepted_upstream": forced
            .into_iter()
            .map(|(stage_id, reason)| json!({"stage_id": stage_id, "reason": reason}))
            .collect::<Vec<_>>(),
        "items": items,
    }))
    .map(Some)
    .map_err(WorkflowError::from)
}

fn emits_empty_items(body: &str) -> bool {
    crate::remediation_items::items_from_text(body).is_some_and(|items| items.is_empty())
}

fn remediation_items_from_dependencies(
    store: &WorkflowStore,
    run: &WorkflowRun,
    deps: &[String],
) -> WorkflowResult<Vec<Value>> {
    let mut items = Vec::new();
    for dep in deps {
        for body in dependency_bodies(store, run, dep)? {
            if let Some(mut parsed) = crate::remediation_items::items_from_text(&body) {
                items.append(&mut parsed);
            }
        }
    }
    Ok(items)
}

fn dependency_bodies(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage_id: &str,
) -> WorkflowResult<Vec<String>> {
    let mut bodies = Vec::new();
    if let Some(state) = run.stages.get(stage_id) {
        for artifact in &state.artifacts {
            bodies.push(read_artifact(store, run, artifact)?);
        }
    }
    for item in run.items.values().filter(|item| item.stage_id == stage_id) {
        if let Some(artifact) = &item.artifact {
            bodies.push(read_artifact(store, run, artifact)?);
        }
    }
    Ok(bodies)
}

fn read_artifact(
    store: &WorkflowStore,
    run: &WorkflowRun,
    artifact: &ArtifactRef,
) -> WorkflowResult<String> {
    let path = store.run_dir(&run.id).join(&artifact.path);
    std::fs::read_to_string(&path).map_err(|err| WorkflowError::io(&path, err))
}

fn dependency_closure(run: &WorkflowRun, stage: &StageSpec) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from(stage.depends_on.clone());
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        out.push(id.clone());
        if let Some(spec_stage) = run.spec.stages.iter().find(|stage| stage.id == id) {
            queue.extend(spec_stage.depends_on.iter().cloned());
        }
    }
    out
}

fn forced_acceptances(run: &WorkflowRun, stage_ids: &[String]) -> Vec<(String, String)> {
    stage_ids
        .iter()
        .filter_map(|stage_id| {
            let state = run.stages.get(stage_id)?;
            (state.status == StageStatus::ForcedAccepted).then(|| {
                (
                    stage_id.clone(),
                    state
                        .error
                        .clone()
                        .unwrap_or_else(|| "forced accepted without stored reason".into()),
                )
            })
        })
        .collect()
}

fn feeds_implementation_fanout(run: &WorkflowRun, stage: &StageSpec) -> bool {
    run.spec.stages.iter().any(|candidate| {
        candidate.effective_item_kind() == StageKind::Implementation
            && candidate
                .foreach
                .as_deref()
                .and_then(foreach_dependency)
                .is_some_and(|dep| dep == stage.id)
    })
}

fn foreach_dependency(foreach: &str) -> Option<&str> {
    let inner = foreach.trim().strip_prefix("${")?.strip_suffix('}')?;
    inner.split('.').next()
}

fn ensure_items_have_targets(stage: &StageSpec, items: &[Value]) -> WorkflowResult<()> {
    if let Some((idx, _)) = items
        .iter()
        .enumerate()
        .find(|(_, item)| !has_target_files(item))
    {
        return Err(WorkflowError::StageFailed(format!(
            "empty remediation inventory '{}' could only synthesize item {} without target_files; refusing unsafe implementation fan-out",
            stage.id, idx
        )));
    }
    Ok(())
}

fn normalize_items(items: Vec<Value>) -> Vec<Value> {
    items
        .into_iter()
        .enumerate()
        .map(|(idx, item)| normalize_item(idx, item))
        .collect()
}

fn normalize_item(idx: usize, item: Value) -> Value {
    match item {
        Value::Object(mut map) => {
            if !has_work_unit_id(&map) {
                let id = map
                    .get("related_task_id")
                    .or_else(|| map.get("finding_id"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("remediation-item-{idx}"));
                map.insert("task_id".to_string(), Value::String(id));
            }
            Value::Object(map)
        }
        other => other,
    }
}

fn has_work_unit_id(map: &serde_json::Map<String, Value>) -> bool {
    [
        "work_unit_id",
        "work_unit_ids",
        "task_id",
        "task_ids",
        "implemented_work_unit_id",
        "implemented_work_unit_ids",
        "implemented_task_ids",
    ]
    .iter()
    .any(|key| map.get(*key).is_some())
}

fn has_target_files(item: &Value) -> bool {
    ["target_files", "expected_target_files"]
        .iter()
        .any(|key| match item.get(*key) {
            Some(Value::String(value)) => !value.trim().is_empty(),
            Some(Value::Array(values)) => values
                .iter()
                .filter_map(Value::as_str)
                .any(|value| !value.trim().is_empty()),
            _ => false,
        })
}

fn dedupe_items(items: Vec<Value>) -> Vec<Value> {
    let mut seen = BTreeSet::new();
    items
        .into_iter()
        .filter(|item| {
            let key = serde_json::to_string(item).unwrap_or_else(|_| item.to_string());
            seen.insert(key)
        })
        .collect()
}
