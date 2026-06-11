use std::fs;

use serde_json::{Value, json};

use crate::error::{WorkflowError, WorkflowResult};
use crate::fanout::{FanoutItem, extract_items};
use crate::reducers::ReducerInput;
use crate::run::{ArtifactRef, WorkflowRun};
use crate::source_context;
use crate::spec::StageSpec;
use crate::store::WorkflowStore;

const MAX_ARTIFACT_CHARS: usize = 32_000;

pub use crate::context_output::{output_reports_blocked, output_reports_failed_verification};

pub fn stage_input(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Value> {
    let root = source_context::effective_root(store, run);
    Ok(json!({
        "workflow_task": run.spec.task,
        "stage_task": stage.task,
        "stage_extra": stage.extra,
        "stage_input": stage.input,
        "target_repository_root": root.display().to_string(),
        "dependencies": dependency_context(store, run, &stage.depends_on)?,
        "source_files": source_context::stage_source_files(store, run, stage),
    }))
}

pub fn fanout_input(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
    item: &FanoutItem,
) -> WorkflowResult<Value> {
    let context = stage_input(store, run, stage)?;
    let sources = source_context::fanout_source_files(store, run, stage, item, &context);
    Ok(json!({
        "workflow_task": run.spec.task,
        "stage_task": stage.task,
        "stage_extra": stage.extra,
        "stage_input": stage.input,
        "target_repository_root": source_context::effective_root(store, run).display().to_string(),
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
    if stage.input.get("items").and_then(Value::as_array).is_some() {
        return Ok(extract_items(stage));
    }
    // A fan-out that declares iteration intent via `foreach: ${producer.items}`
    // must resolve to real structured items from its producer. If it does not,
    // fail fast instead of collapsing to a single synthetic item that the agent
    // would (correctly) reject as missing evidence.
    if let Some(dep) = foreach_dependency(stage) {
        let allow_empty = fanout_allows_empty_items(stage);
        return match dependency_items(store, run, stage)? {
            Some(items) if !items.is_empty() || allow_empty => Ok(items),
            Some(_) => Err(WorkflowError::InvalidFanout(format!(
                "fanout stage '{}' declares `foreach` over '{dep}' but that producer emitted an empty `items:` list",
                stage.id
            ))),
            None => Err(WorkflowError::InvalidFanout(format!(
                "fanout stage '{}' declares `foreach` over '{dep}' but that producer emitted no parseable `items:` structure",
                stage.id
            ))),
        };
    }
    let files = source_context::stage_source_files(store, run, stage);
    if let Some(items) = source_file_items(stage, files) {
        return Ok(items);
    }
    Ok(extract_items(stage))
}

pub fn reducer_inputs(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Vec<ReducerInput>> {
    stage
        .depends_on
        .iter()
        .map(|dep| {
            let bodies = dependency_bodies(store, run, dep)?;
            Ok(ReducerInput {
                stage_id: dep.clone(),
                content: bodies.join("\n\n"),
                accepted: run.accepted_stage(dep),
                failed: !run.accepted_stage(dep),
            })
        })
        .collect()
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
            if let Some(reason) = output_reports_blocked(&body) {
                return Ok(Some(format!(
                    "dependency `{dep}` reported blocked: {reason}"
                )));
            }
            if let Some(reason) = output_reports_failed_verification(&body) {
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
    run.stages
        .get(stage_id)
        .into_iter()
        .flat_map(|stage| &stage.artifacts)
        .map(|artifact| artifact_value(store, run, artifact))
        .collect::<WorkflowResult<Vec<_>>>()
}

fn artifact_value(
    store: &WorkflowStore,
    run: &WorkflowRun,
    artifact: &ArtifactRef,
) -> WorkflowResult<Value> {
    let path = store.run_dir(&run.id).join(&artifact.path);
    let body = fs::read_to_string(&path).map_err(|e| WorkflowError::io(&path, e))?;
    Ok(json!({
        "id": artifact.id,
        "path": artifact.path.display().to_string(),
        "content_hash": artifact.content_hash,
        "accepted": artifact.accepted,
        "content": truncate_chars(&body, MAX_ARTIFACT_CHARS),
    }))
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

fn dependency_items(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Option<Vec<FanoutItem>>> {
    let Some(dep) = foreach_dependency(stage) else {
        return Ok(None);
    };
    for body in dependency_bodies(store, run, &dep)? {
        if let Some(items) = parse_items(&body) {
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

fn parse_items(body: &str) -> Option<Vec<Value>> {
    candidate_documents(body)
        .into_iter()
        .find_map(parse_items_doc)
}

fn parse_items_doc(body: &str) -> Option<Vec<Value>> {
    serde_json::from_str::<Value>(body)
        .ok()
        .or_else(|| serde_yaml_ng::from_str::<Value>(body).ok())
        .and_then(|value| value.get("items").and_then(Value::as_array).cloned())
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
