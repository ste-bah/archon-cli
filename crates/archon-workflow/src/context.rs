use std::fs;

use serde_json::{Value, json};

use crate::error::{WorkflowError, WorkflowResult};
use crate::fanout::{FanoutItem, extract_items};
use crate::item_filter;
use crate::run::{ArtifactRef, WorkflowRun};
use crate::source_context;
use crate::spec::{StageKind, StageSpec};
use crate::store::WorkflowStore;

const MAX_ARTIFACT_CHARS: usize = 32_000;

pub use crate::context_output::{
    output_reports_blocked, output_reports_failed_verification, output_reports_zero_matched_tests,
};

pub fn stage_input(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Value> {
    let root = stage_target_root(store, run, stage)?;
    Ok(json!({
        "workflow_task": run.spec.task,
        "stage_task": stage.task,
        "verify_command": stage.verify_command,
        "stage_extra": stage.extra,
        "stage_input": stage.input,
        "target_repository_root": root.display().to_string(),
        "dependencies": dependency_context(store, run, &stage.depends_on)?,
        "source_files": source_context::stage_source_files(store, run, stage),
    }))
}

fn stage_target_root(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<std::path::PathBuf> {
    if stage.kind == StageKind::Implementation {
        return source_context::implementation_root_for_payload_targets(
            store,
            run,
            &stage.input,
            &stage.expected_target_files,
        );
    }
    Ok(source_context::effective_root(store, run))
}

pub fn fanout_input(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
    item: &FanoutItem,
) -> WorkflowResult<Value> {
    let context = stage_input(store, run, stage)?;
    let sources = source_context::fanout_source_files(store, run, stage, item, &context);
    let target_root = source_context::fanout_item_target_root(
        store,
        run,
        &item.payload,
        &stage.expected_target_files,
    );
    Ok(json!({
        "workflow_task": run.spec.task,
        "stage_task": stage.task,
        "verify_command": stage.verify_command,
        "stage_extra": stage.extra,
        "stage_input": stage.input,
        "target_repository_root": target_root.display().to_string(),
        "dependencies": context.get("dependencies").cloned().unwrap_or_else(|| json!([])),
        "source_files": sources,
        "fanout_stage": stage.id,
        "fanout_item_id": item.id,
        "fanout_item": item.payload,
        "context": context,
    }))
}

pub fn fanout_items(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Vec<FanoutItem>> {
    let allow_empty = fanout_allows_empty_items(stage)
        || crate::completion_proof::has_empty_completion_contract(&run.spec, stage);
    if stage.input.get("items").and_then(Value::as_array).is_some() {
        let items = item_filter::apply_stage_filter(stage, extract_items(stage), allow_empty)?;
        return validated_fanout_items(stage, items);
    }
    // A fan-out that declares iteration intent via `foreach: ${producer.items}`
    // must resolve to real structured items from its producer. If it does not,
    // fail fast instead of collapsing to a single synthetic item that the agent
    // would (correctly) reject as missing evidence.
    if let Some(dep) = foreach_dependency(stage) {
        let items = match dependency_items(store, run, stage)? {
            Some(items) if !items.is_empty() || allow_empty => items,
            Some(_) => Err(WorkflowError::InvalidFanout(format!(
                "fanout stage '{}' declares `foreach` over '{dep}' but that producer emitted an empty `items:` list",
                stage.id
            )))?,
            None => Err(WorkflowError::InvalidFanout(format!(
                "fanout stage '{}' declares `foreach` over '{dep}' but that producer emitted no parseable `items:` structure",
                stage.id
            )))?,
        };
        let items = item_filter::apply_stage_filter(stage, items, allow_empty)?;
        return validated_fanout_items(stage, items);
    }
    let files = source_context::stage_source_files(store, run, stage);
    if let Some(items) = source_file_items(stage, files) {
        let items = item_filter::apply_stage_filter(stage, items, allow_empty)?;
        return validated_fanout_items(stage, items);
    }
    let items = item_filter::apply_stage_filter(stage, extract_items(stage), allow_empty)?;
    validated_fanout_items(stage, items)
}

pub fn quality_gate_failure(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Option<String>> {
    if stage.depends_on.is_empty() {
        return Ok(None);
    }
    let mut saw_artifact = false;
    for dep in &stage.depends_on {
        for body in dependency_bodies(store, run, dep)? {
            saw_artifact = true;
            if let Some(reason) =
                crate::completion_proof::invalid_completed_items_reason(dep, &body)
            {
                return Ok(Some(format!(
                    "dependency `{dep}` failed completed-items contract: {reason}"
                )));
            }
            if let Some(reason) = output_reports_blocked(&body) {
                return Ok(Some(format!(
                    "dependency `{dep}` reported blocked: {reason}"
                )));
            }
            if let Some(reason) = dependency_failed_verification(run, dep, &body) {
                return Ok(Some(format!(
                    "dependency `{dep}` reported failed verification: {reason}"
                )));
            }
        }
    }
    if saw_artifact {
        Ok(None)
    } else {
        Ok(Some(
            "quality gate has no upstream artifacts to inspect".into(),
        ))
    }
}

fn dependency_failed_verification(run: &WorkflowRun, stage_id: &str, body: &str) -> Option<String> {
    let dependency = run.spec.stages.iter().find(|stage| stage.id == stage_id);
    let Some(dependency) = dependency else {
        return crate::context_output::output_reports_failed_execution(body);
    };
    if dependency.write_capable() {
        return output_reports_failed_verification(body);
    }
    if dependency_is_verification_like(dependency) {
        return crate::context_output::output_reports_failed_execution(body);
    }
    crate::context_output::output_reports_failed_execution_without_test_counts(body)
}

fn dependency_is_verification_like(stage: &StageSpec) -> bool {
    if stage.verify_command.is_some() {
        return true;
    }
    if stage.kind == StageKind::Fanout && stage.effective_item_kind() != StageKind::Implementation {
        return false;
    }
    let text =
        format!("{} {}", stage.id, stage.task.as_deref().unwrap_or_default()).to_ascii_lowercase();
    let review_like = ["review", "audit", "critic", "adversarial"]
        .iter()
        .any(|needle| text.contains(needle));
    let verification_like = [
        "test",
        "tests",
        "verification",
        "verify",
        "clippy",
        "lint",
        "fmt",
        "build",
        "check",
    ]
    .iter()
    .any(|needle| text.contains(needle));
    !review_like && verification_like
}

fn dependency_context(
    store: &WorkflowStore,
    run: &WorkflowRun,
    deps: &[String],
) -> WorkflowResult<Vec<Value>> {
    deps.iter()
        .map(|dep| {
            let artifacts = dependency_artifacts(store, run, dep)?;
            Ok(json!({
                "stage_id": dep,
                "accepted": run.accepted_stage(dep),
                "artifacts": artifacts,
            }))
        })
        .collect()
}

fn dependency_artifacts(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage_id: &str,
) -> WorkflowResult<Vec<Value>> {
    let mut artifacts = run
        .stages
        .get(stage_id)
        .into_iter()
        .flat_map(|stage| &stage.artifacts)
        .map(|artifact| artifact_value(store, run, artifact))
        .collect::<WorkflowResult<Vec<_>>>()?;
    for item in run.items.values().filter(|item| item.stage_id == stage_id) {
        if let Some(artifact) = &item.artifact {
            artifacts.push(artifact_value(store, run, artifact)?);
        }
    }
    Ok(artifacts)
}

fn artifact_value(
    store: &WorkflowStore,
    run: &WorkflowRun,
    artifact: &ArtifactRef,
) -> WorkflowResult<Value> {
    let body = artifact_body(store, run, artifact)?;
    Ok(json!({
        "id": artifact.id,
        "path": artifact.path.display().to_string(),
        "content_hash": artifact.content_hash,
        "accepted": artifact.accepted,
        "content": truncate_chars(&body, MAX_ARTIFACT_CHARS),
    }))
}

fn artifact_body(
    store: &WorkflowStore,
    run: &WorkflowRun,
    artifact: &ArtifactRef,
) -> WorkflowResult<String> {
    let path = store.run_dir(&run.id).join(&artifact.path);
    fs::read_to_string(&path).map_err(|e| WorkflowError::io(&path, e))
}

fn dependency_bodies(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage_id: &str,
) -> WorkflowResult<Vec<String>> {
    Ok(dependency_artifacts(store, run, stage_id)?
        .into_iter()
        .filter_map(|artifact| {
            artifact
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

fn dependency_item_bodies(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage_id: &str,
) -> WorkflowResult<Vec<String>> {
    let mut bodies = run
        .stages
        .get(stage_id)
        .into_iter()
        .flat_map(|stage| &stage.artifacts)
        .map(|artifact| artifact_body(store, run, artifact))
        .collect::<WorkflowResult<Vec<_>>>()?;
    for item in run.items.values().filter(|item| item.stage_id == stage_id) {
        if let Some(artifact) = &item.artifact {
            bodies.push(artifact_body(store, run, artifact)?);
        }
    }
    Ok(bodies)
}

fn dependency_items(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Option<Vec<FanoutItem>>> {
    let Some(dep) = foreach_dependency(stage) else {
        return Ok(None);
    };
    for body in dependency_item_bodies(store, run, &dep)? {
        if let Some(items) =
            parse_items(&body).or_else(|| crate::remediation_items::items_from_text(&body))
        {
            let items = items
                .into_iter()
                .enumerate()
                .map(|(idx, payload)| FanoutItem {
                    id: format!("{}-{idx}", stage.id),
                    payload: source_context::enrich_payload(store, run, payload),
                })
                .collect::<Vec<_>>();
            return Ok(Some(items));
        }
    }
    Ok(None)
}

fn fanout_allows_empty_items(stage: &StageSpec) -> bool {
    stage
        .extra
        .get("allow_empty_items")
        .or_else(|| stage.input.get("allow_empty_items"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || crate::completion_proof::enabled(stage)
}

fn source_file_items(stage: &StageSpec, files: Vec<Value>) -> Option<Vec<FanoutItem>> {
    (!files.is_empty()).then(|| {
        files
            .into_iter()
            .enumerate()
            .map(|(idx, payload)| FanoutItem {
                id: format!("{}-{idx}", stage.id),
                payload,
            })
            .collect()
    })
}

fn validated_fanout_items(
    stage: &StageSpec,
    items: Vec<FanoutItem>,
) -> WorkflowResult<Vec<FanoutItem>> {
    if stage.effective_item_kind() != StageKind::Implementation || items.is_empty() {
        return Ok(items);
    }
    let stage_units = crate::work_unit_coverage::stage_required_units(stage);
    let requires_item_work_units = !stage
        .extra
        .get("artifact_self_heal")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let has_stage_work_units = !stage_units.is_empty();
    for item in &items {
        if requires_item_work_units
            && crate::work_unit_coverage::item_required_units(&item.payload).is_empty()
            && !has_stage_work_units
        {
            return Err(WorkflowError::InvalidFanout(format!(
                "implementation fanout stage '{}' item '{}' requires work_unit_id, work_unit_ids, task_id, or task_ids",
                stage.id, item.id
            )));
        }
        if !item_has_declared_targets(stage, &item.payload) {
            return Err(WorkflowError::InvalidFanout(format!(
                "implementation fanout stage '{}' item '{}' requires target_files or expected_target_files before write execution",
                stage.id, item.id
            )));
        }
    }
    Ok(items)
}

fn item_has_declared_targets(stage: &StageSpec, payload: &Value) -> bool {
    !stage.expected_target_files.is_empty()
        || has_string_or_string_array(payload.get("target_files"))
        || has_string_or_string_array(payload.get("expected_target_files"))
}

fn has_string_or_string_array(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .any(|text| !text.trim().is_empty()),
        _ => false,
    }
}

fn parse_items(body: &str) -> Option<Vec<Value>> {
    candidate_documents(body)
        .into_iter()
        .find_map(parse_items_doc)
}

fn parse_items_doc(body: &str) -> Option<Vec<Value>> {
    serde_json::from_str::<Value>(body)
        .ok()
        .or_else(|| serde_yaml_ng::from_str::<Value>(body).ok())
        .and_then(|value| {
            value
                .get("items")
                .and_then(Value::as_array)
                .cloned()
                .or_else(|| {
                    value
                        .get("completed_items")
                        .and_then(Value::as_array)
                        .map(|_| Vec::new())
                })
        })
}

fn candidate_documents(body: &str) -> Vec<&str> {
    let mut docs = vec![body.trim()];
    let mut rest = body;
    while let Some(start) = rest.find("```") {
        rest = &rest[start + 3..];
        if let Some(newline) = rest.find('\n') {
            rest = &rest[newline + 1..];
        }
        let Some(end) = rest.find("```") else {
            break;
        };
        docs.push(rest[..end].trim());
        rest = &rest[end + 3..];
    }
    docs
}

fn foreach_dependency(stage: &StageSpec) -> Option<String> {
    let foreach = stage.foreach.as_deref()?.trim();
    let inner = foreach.strip_prefix("${")?.strip_suffix('}')?;
    inner.split('.').next().map(str::to_string)
}

fn truncate_chars(body: &str, limit: usize) -> String {
    if body.chars().count() <= limit {
        return body.to_string();
    }
    let mut truncated = body.chars().take(limit).collect::<String>();
    truncated.push_str("\n\n[truncated]");
    truncated
}
