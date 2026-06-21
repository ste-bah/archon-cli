use std::collections::BTreeSet;

use crate::context;
use crate::error::{WorkflowError, WorkflowResult};
use crate::run::{StageStatus, WorkflowRun};
use crate::runner::StageRunOutput;
use crate::spec::StageSpec;
use crate::store::WorkflowStore;

pub(super) fn empty_fanout_result(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
    implementation_items: bool,
) -> WorkflowResult<StageRunOutput> {
    if fanout_can_noop(store, run, stage)? {
        return Ok(StageRunOutput::markdown(format!(
            "Fan-out stage `{}` completed 0 item(s) as an explicit recovery no-op.",
            stage.id
        )));
    }
    if let Some(summary) = context::empty_completion_summary(store, run, stage)? {
        return Ok(StageRunOutput::markdown(format!(
            "Fan-out stage `{}` completed 0 item(s): {summary}.",
            stage.id
        )));
    }
    let kind = if implementation_items {
        "implementation fan-out"
    } else {
        "fan-out"
    };
    Err(WorkflowError::StageFailed(format!(
        "{kind} stage '{}' resolved zero items; only explicit recovery/remediation fan-outs may no-op",
        stage.id
    )))
}

fn fanout_can_noop(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<bool> {
    if crate::remediation_noop::empty_remediation_noop_reason(store, run, stage)?.is_some() {
        return Ok(true);
    }
    if !(fanout_allows_empty_items(stage) && crate::stage::is_recovery_stage(stage)) {
        return Ok(false);
    }
    if let Some((stage_id, reason)) = unresolved_forced_acceptance(run, stage) {
        return Err(WorkflowError::StageFailed(format!(
            "recovery fan-out '{}' resolved zero items while upstream stage '{}' still has unresolved forced-accepted failure: {}",
            stage.id, stage_id, reason
        )));
    }
    Ok(true)
}

fn fanout_allows_empty_items(stage: &StageSpec) -> bool {
    stage
        .extra
        .get("allow_empty_items")
        .or_else(|| stage.input.get("allow_empty_items"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn unresolved_forced_acceptance(run: &WorkflowRun, stage: &StageSpec) -> Option<(String, String)> {
    let mut seen = BTreeSet::new();
    for dep in &stage.depends_on {
        if let Some(found) = unresolved_forced_acceptance_from(run, dep, &mut seen) {
            return Some(found);
        }
    }
    None
}

fn unresolved_forced_acceptance_from(
    run: &WorkflowRun,
    stage_id: &str,
    seen: &mut BTreeSet<String>,
) -> Option<(String, String)> {
    if !seen.insert(stage_id.to_string()) {
        return None;
    }
    if let Some(state) = run.stages.get(stage_id)
        && state.status == StageStatus::ForcedAccepted
        && let Some(reason) = state.error.as_ref()
    {
        return Some((stage_id.to_string(), reason.clone()));
    }
    let spec_stage = run.spec.stages.iter().find(|stage| stage.id == stage_id)?;
    for dep in &spec_stage.depends_on {
        if let Some(found) = unresolved_forced_acceptance_from(run, dep, seen) {
            return Some(found);
        }
    }
    None
}
