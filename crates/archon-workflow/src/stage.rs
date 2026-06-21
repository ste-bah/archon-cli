use std::collections::{BTreeMap, BTreeSet};

use crate::error::{WorkflowError, WorkflowResult};
use crate::run::{StageStatus, WorkflowRun};
use crate::spec::{StageKind, StageSpec, WorkflowSpec};

pub fn ordered_stages(spec: &WorkflowSpec) -> WorkflowResult<Vec<StageSpec>> {
    let mut remaining: BTreeMap<String, StageSpec> = spec
        .stages
        .iter()
        .map(|stage| (stage.id.clone(), stage.clone()))
        .collect();
    let mut accepted = BTreeSet::new();
    let mut ordered = Vec::new();
    while !remaining.is_empty() {
        let ready: Vec<String> = remaining
            .iter()
            .filter(|(_, stage)| stage.depends_on.iter().all(|dep| accepted.contains(dep)))
            .map(|(id, _)| id.clone())
            .collect();
        if ready.is_empty() {
            return Err(WorkflowError::DependencyCycle(
                remaining.keys().cloned().collect(),
            ));
        }
        for id in ready {
            if let Some(stage) = remaining.remove(&id) {
                accepted.insert(id);
                ordered.push(stage);
            }
        }
    }
    Ok(ordered)
}

pub fn stage_ready(run: &WorkflowRun, stage: &StageSpec) -> bool {
    run.stages
        .get(&stage.id)
        .is_some_and(|state| state.status == StageStatus::Pending)
        && stage
            .depends_on
            .iter()
            .all(|dep| run.dependency_satisfied_stage(dep))
}

pub fn stage_feeds_downstream_recovery(spec: &WorkflowSpec, stage_id: &str) -> bool {
    let Some(source) = spec.stages.iter().find(|stage| stage.id == stage_id) else {
        return false;
    };
    if recovery_stage_must_stop_on_failure(source) {
        return false;
    }
    spec.stages.iter().any(|stage| {
        is_recovery_stage(stage)
            && dependency_closure_contains(spec, &stage.id, stage_id, &mut BTreeSet::new())
    })
}

pub fn stage_failure_feeds_downstream_recovery(
    spec: &WorkflowSpec,
    stage_id: &str,
    err: &WorkflowError,
) -> bool {
    stage_feeds_downstream_recovery(spec, stage_id) && recoverable_stage_failure(err)
}

fn recoverable_stage_failure(err: &WorkflowError) -> bool {
    let WorkflowError::StageFailed(reason) = err else {
        return false;
    };
    let lower = reason.to_ascii_lowercase();
    ![
        "accepted status without required evidence fields",
        "agent output asks for confirmation",
        "resolved zero items",
        "empty remediation inventory",
        "invalid fan-out",
        "declares `foreach`",
        "producer emitted no parseable",
        "missing output for",
        "read manifest",
        "parse manifest",
        "source input hash changed",
        "artifact cannot be reused",
        "workflow state is corrupt",
        "policy denied",
        "provider tier",
        "recordprompt",
        "malformed",
        "schema",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn recovery_stage_must_stop_on_failure(stage: &StageSpec) -> bool {
    is_recovery_stage(stage) && matches!(stage.kind, StageKind::Implementation | StageKind::Fanout)
}

pub fn is_recovery_stage(stage: &StageSpec) -> bool {
    if stage
        .extra
        .get("failure_aware")
        .or_else(|| stage.extra.get("artifact_self_heal"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    let can_create_repair_work = crate::spec::stage_declares_items_producer(stage)
        || matches!(stage.kind, StageKind::Implementation)
        || (matches!(stage.kind, StageKind::Fanout)
            && stage.item_kind == Some(StageKind::Implementation));
    let text = format!(
        "{} {}",
        stage.id.to_ascii_lowercase(),
        stage
            .task
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase()
    );
    [
        "remediation",
        "remediate",
        "repair",
        "self-heal",
        "self heal",
    ]
    .iter()
    .any(|needle| text.contains(needle))
        && can_create_repair_work
}

fn dependency_closure_contains(
    spec: &WorkflowSpec,
    stage_id: &str,
    target: &str,
    seen: &mut BTreeSet<String>,
) -> bool {
    if !seen.insert(stage_id.to_string()) {
        return false;
    }
    let Some(stage) = spec.stages.iter().find(|stage| stage.id == stage_id) else {
        return false;
    };
    stage
        .depends_on
        .iter()
        .any(|dep| dep == target || dependency_closure_contains(spec, dep, target, seen))
}

pub fn source_input_hash(stage: &StageSpec) -> String {
    let body = match serde_json::to_vec(stage) {
        Ok(body) => body,
        Err(_) => stage.id.as_bytes().to_vec(),
    };
    blake3::hash(&body).to_hex().to_string()
}
