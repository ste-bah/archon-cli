use serde_json::{Value, json};

use crate::error::{WorkflowError, WorkflowResult};
use crate::persistence;
use crate::run::{ArtifactRef, StageStatus, WorkflowRun};
use crate::runner::StageRunOutput;
use crate::spec::StageSpec;
use crate::store::WorkflowStore;

pub(crate) fn attach_agent_noop_if_empty(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Option<StageRunOutput>> {
    let Some(reason) = empty_remediation_noop_reason(store, run, stage)? else {
        return Ok(None);
    };
    let body = noop_body(stage, &reason);
    let output = StageRunOutput {
        body,
        extension: "json".into(),
        provider_id: Some("workflow-engine".into()),
        resolved_model: Some("deterministic-remediation-noop".into()),
        tokens_in: 0,
        tokens_out: 0,
        cost_usd: 0.0,
        tool_uses: Vec::new(),
    };
    let artifact = persistence::write_attached_stage_artifact(
        store,
        run,
        stage,
        &stage.id,
        &output.extension,
        output.body.clone(),
        true,
    )?;
    persistence::record_agent_output(
        store,
        &run.id,
        &stage.id,
        &stage.id,
        Some(&output),
        Some(&artifact),
        true,
        None,
    )?;
    Ok(Some(output))
}

pub(crate) fn empty_remediation_noop_reason(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Option<String>> {
    if !is_post_remediation_stage(stage) || !allows_empty_remediation_noop(stage) {
        return Ok(None);
    }
    let deps = dependency_closure(run, stage);
    if let Some((stage_id, reason)) = unresolved_forced_acceptance(run, &deps) {
        return Err(WorkflowError::StageFailed(format!(
            "post-remediation no-op for '{}' blocked by unresolved forced-accepted upstream stage '{}': {}",
            stage.id, stage_id, reason
        )));
    }
    for dep in deps {
        let Some(dep_stage) = run.spec.stages.iter().find(|candidate| candidate.id == dep) else {
            continue;
        };
        if !is_remediation_inventory(dep_stage) {
            continue;
        }
        if dependency_has_empty_items(store, run, &dep_stage.id)? {
            return Ok(Some(format!(
                "remediation inventory `{}` emitted an accepted empty items list",
                dep_stage.id
            )));
        }
    }
    Ok(None)
}

fn noop_body(stage: &StageSpec, reason: &str) -> String {
    serde_json::to_string_pretty(&json!({
        "schema": "archon.workflow.remediation_noop.v1",
        "stage": stage.id,
        "status": "verified",
        "remediation_noop": true,
        "reason": reason,
        "commands_run": [],
        "residual_gaps": [],
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn is_post_remediation_stage(stage: &StageSpec) -> bool {
    let text = stage_text(stage);
    text.contains("post_remediation")
        || text.contains("post-remediation")
        || text.contains("post remediation")
}

fn is_remediation_inventory(stage: &StageSpec) -> bool {
    let text = stage_text(stage);
    text.contains("remediation_inventory")
        || text.contains("remediation-inventory")
        || text.contains("remediation inventory")
}

fn allows_empty_remediation_noop(stage: &StageSpec) -> bool {
    stage
        .extra
        .get("allow_empty_remediation_noop")
        .or_else(|| stage.input.get("allow_empty_remediation_noop"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn stage_text(stage: &StageSpec) -> String {
    format!(
        "{} {}",
        stage.id.to_ascii_lowercase(),
        stage
            .task
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase()
    )
}

fn dependency_closure(run: &WorkflowRun, stage: &StageSpec) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = stage.depends_on.clone();
    while let Some(id) = stack.pop() {
        if out.contains(&id) {
            continue;
        }
        out.push(id.clone());
        if let Some(spec_stage) = run.spec.stages.iter().find(|candidate| candidate.id == id) {
            stack.extend(spec_stage.depends_on.iter().cloned());
        }
    }
    out
}

fn unresolved_forced_acceptance(
    run: &WorkflowRun,
    stage_ids: &[String],
) -> Option<(String, String)> {
    stage_ids.iter().find_map(|stage_id| {
        let state = run.stages.get(stage_id)?;
        if state.status == StageStatus::ForcedAccepted {
            return state
                .error
                .as_ref()
                .map(|reason| (stage_id.clone(), reason.clone()));
        }
        None
    })
}

fn dependency_has_empty_items(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage_id: &str,
) -> WorkflowResult<bool> {
    for body in dependency_bodies(store, run, stage_id)? {
        if parse_items_array(&body).is_some_and(|items| items.is_empty()) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn dependency_bodies(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage_id: &str,
) -> WorkflowResult<Vec<String>> {
    let mut bodies = Vec::new();
    if let Some(stage) = run.stages.get(stage_id) {
        for artifact in &stage.artifacts {
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

fn parse_items_array(body: &str) -> Option<Vec<Value>> {
    candidate_documents(body).into_iter().find_map(|doc| {
        serde_json::from_str::<Value>(doc)
            .ok()
            .or_else(|| serde_yaml_ng::from_str::<Value>(doc).ok())
            .and_then(|value| value.get("items").and_then(Value::as_array).cloned())
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
