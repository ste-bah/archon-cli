use std::path::PathBuf;

use chrono::Utc;
use serde_json::{Value, json};

use crate::error::WorkflowResult;
use crate::events::sanitize_value;
use crate::run::ArtifactRef;
use crate::runner::{StageRunOutput, StageRunRequest};
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
    record_agent_output_with_status(
        store,
        run_id,
        stage_id,
        item_id,
        output,
        artifact,
        if accepted { "accepted" } else { "failed" },
        accepted,
        error,
    )
}

pub(crate) fn record_captured_agent_output(
    store: &WorkflowStore,
    run_id: &str,
    stage_id: &str,
    item_id: &str,
    output: &StageRunOutput,
) -> WorkflowResult<()> {
    record_agent_output_with_status(
        store,
        run_id,
        stage_id,
        item_id,
        Some(output),
        None,
        "captured",
        false,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn record_agent_output_with_status(
    store: &WorkflowStore,
    run_id: &str,
    stage_id: &str,
    item_id: &str,
    output: Option<&StageRunOutput>,
    artifact: Option<&ArtifactRef>,
    status: &str,
    accepted: bool,
    error: Option<&str>,
) -> WorkflowResult<()> {
    let record = sanitize_value(json!({
        "schema": "archon.workflow.agent_output.v1",
        "run_id": run_id,
        "stage_id": stage_id,
        "item_id": item_id,
        "status": status,
        "accepted": accepted,
        "provider": output.and_then(|o| o.provider_id.as_deref()),
        "model": output.and_then(|o| o.resolved_model.as_deref()),
        "tokens_in": output.map_or(0, |o| o.tokens_in),
        "tokens_out": output.map_or(0, |o| o.tokens_out),
        "cost_usd": output.map_or(0.0, |o| o.cost_usd),
        "recent_public_tool_calls": output
            .map(|o| o.tool_uses.iter().take(20).cloned().collect::<Vec<_>>())
            .unwrap_or_default(),
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
