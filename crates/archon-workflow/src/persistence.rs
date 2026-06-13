use std::path::PathBuf;

use chrono::Utc;
use serde_json::{Value, json};

use crate::error::{WorkflowError, WorkflowResult};
use crate::events::sanitize_value;
use crate::reducers::{ReducerInput, ReducerOutput};
use crate::run::{ArtifactRef, WorkflowRun};
use crate::runner::{StageRunOutput, StageRunRequest};
use crate::spec::{ReducerKind, StageSpec};
use crate::stage::source_input_hash;
use crate::store::{WorkflowStore, safe_path_component};

pub(crate) fn record_prompt(
    store: &WorkflowStore,
    request: &StageRunRequest,
) -> WorkflowResult<()> {
    let stage_id = record_stage_id(&request.stage_id, &request.input);
    let item_id = record_item_id(&request.stage_id, &request.input);
    let prompt_hash = hash_json(&json!({
        "task": request.task,
        "input": request.input,
        "agent": request.agent,
        "provider_tier": request.provider_tier,
    }));
    let record = sanitize_value(json!({
        "schema": "archon.workflow.prompt.v1",
        "run_id": request.run_id,
        "stage_id": stage_id,
        "item_id": item_id,
        "stage_kind": request.stage_kind,
        "agent": request.agent,
        "task": request.task,
        "attempt": request.attempt,
        "provider_tier": request.provider_tier,
        "depends_on": request.depends_on,
        "prompt_hash": prompt_hash,
        "input_hash": hash_json(&request.input),
        "created_at": Utc::now().to_rfc3339(),
    }));
    store.write_run_json(
        &request.run_id,
        record_path("prompts", &stage_id, &item_id, "json"),
        &record,
    )
}

pub(crate) fn write_stage_artifact(
    store: &WorkflowStore,
    run_id: &str,
    stage: &StageSpec,
    artifact_key: &str,
    extension: &str,
    body: &str,
    accepted: bool,
) -> WorkflowResult<ArtifactRef> {
    store.write_named_artifact(
        run_id,
        &stage.id,
        artifact_key,
        &source_input_hash(stage),
        extension,
        body.as_bytes(),
        accepted,
    )
}

pub(crate) fn write_attached_stage_artifact(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    artifact_key: &str,
    extension: &str,
    body: String,
    accepted: bool,
) -> WorkflowResult<ArtifactRef> {
    let artifact = write_stage_artifact(
        store,
        &run.id,
        stage,
        artifact_key,
        extension,
        &body,
        accepted,
    )?;
    let state = run
        .stage_mut(&stage.id)
        .ok_or_else(|| WorkflowError::SpecInvalid(format!("missing stage {}", stage.id)))?;
    state.artifacts.push(artifact.clone());
    Ok(artifact)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_agent_output(
    store: &WorkflowStore,
    run_id: &str,
    stage_id: &str,
    item_id: &str,
    output: Option<&StageRunOutput>,
    artifact: Option<&ArtifactRef>,
    accepted: bool,
    error: Option<&str>,
) -> WorkflowResult<()> {
    let record = sanitize_value(json!({
        "schema": "archon.workflow.agent_output.v1",
        "run_id": run_id,
        "stage_id": stage_id,
        "item_id": item_id,
        "status": if accepted { "accepted" } else { "failed" },
        "accepted": accepted,
        "provider": output.and_then(|o| o.provider_id.as_deref()),
        "model": output.and_then(|o| o.resolved_model.as_deref()),
        "tokens_in": output.map_or(0, |o| o.tokens_in),
        "tokens_out": output.map_or(0, |o| o.tokens_out),
        "cost_usd": output.map_or(0.0, |o| o.cost_usd),
        "artifact": artifact.map(artifact_json),
        "body": output.map(|o| public_body(&o.body)),
        "error": error,
        "created_at": Utc::now().to_rfc3339(),
    }));
    store.write_run_json(
        run_id,
        record_path("agent-outputs", stage_id, item_id, "json"),
        &record,
    )
}

pub(crate) fn agent_output_exists(
    store: &WorkflowStore,
    run_id: &str,
    stage_id: &str,
    item_id: &str,
) -> bool {
    store
        .run_dir(run_id)
        .join(record_path("agent-outputs", stage_id, item_id, "json"))
        .exists()
}

pub(crate) fn record_reducer(
    store: &WorkflowStore,
    run_id: &str,
    stage: &StageSpec,
    reducer: ReducerKind,
    inputs: &[ReducerInput],
    output: &ReducerOutput,
    artifact: &ArtifactRef,
) -> WorkflowResult<()> {
    let record = sanitize_value(json!({
        "schema": "archon.workflow.reducer.v1",
        "run_id": run_id,
        "stage_id": stage.id,
        "reducer": reducer,
        "input_count": inputs.len(),
        "inputs": inputs,
        "output": output,
        "artifact": artifact_json(artifact),
        "created_at": Utc::now().to_rfc3339(),
    }));
    let path = PathBuf::from("reducers").join(format!("{}.json", safe_path_component(&stage.id)));
    store.write_run_json(run_id, path, &record)
}

pub(crate) fn record_quality(
    store: &WorkflowStore,
    run_id: &str,
    stage: &StageSpec,
    status: &str,
    reason: Option<&str>,
    artifact: Option<&ArtifactRef>,
) -> WorkflowResult<()> {
    let record = sanitize_value(json!({
        "schema": "archon.workflow.quality.v1",
        "run_id": run_id,
        "stage_id": stage.id,
        "status": status,
        "checked_dependencies": stage.depends_on,
        "reason": reason,
        "artifact": artifact.map(artifact_json),
        "created_at": Utc::now().to_rfc3339(),
    }));
    let path = PathBuf::from("quality").join(format!("{}.json", safe_path_component(&stage.id)));
    store.write_run_json(run_id, path, &record)
}

pub(crate) fn record_forced_accept(
    store: &WorkflowStore,
    run_id: &str,
    stage_id: &str,
    forced_by: &str,
    rationale: &str,
    source: &str,
) -> WorkflowResult<()> {
    let record = sanitize_value(json!({
        "schema": "archon.workflow.forced_accept.v1",
        "run_id": run_id,
        "stage_id": stage_id,
        "status": "forced_accepted",
        "forced_by": forced_by,
        "rationale": rationale,
        "source": source,
        "created_at": Utc::now().to_rfc3339(),
    }));
    let path = PathBuf::from("quality")
        .join("forced")
        .join(format!("{}.json", safe_path_component(stage_id)));
    store.write_run_json(run_id, path, &record)
}

fn record_stage_id(default_stage_id: &str, input: &Value) -> String {
    input
        .get("fanout_stage")
        .and_then(Value::as_str)
        .unwrap_or(default_stage_id)
        .to_string()
}

fn record_item_id(default_stage_id: &str, input: &Value) -> String {
    input
        .get("fanout_item_id")
        .and_then(Value::as_str)
        .unwrap_or(default_stage_id)
        .to_string()
}

fn record_path(root: &str, stage_id: &str, item_id: &str, extension: &str) -> PathBuf {
    PathBuf::from(root)
        .join(safe_path_component(stage_id))
        .join(format!(
            "{}.{}",
            safe_path_component(item_id),
            extension.trim_start_matches('.')
        ))
}

fn artifact_json(artifact: &ArtifactRef) -> Value {
    json!({
        "id": artifact.id,
        "path": artifact.path.display().to_string(),
        "content_hash": artifact.content_hash,
        "producing_stage": artifact.producing_stage,
        "source_input_hash": artifact.source_input_hash,
        "accepted": artifact.accepted,
    })
}

fn public_body(body: &str) -> Value {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        sanitize_value(value)
    } else {
        Value::String(body.to_string())
    }
}

fn hash_json(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    blake3::hash(&bytes).to_hex().to_string()
}
