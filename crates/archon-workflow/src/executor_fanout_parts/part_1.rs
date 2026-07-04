use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::acceptance;
use crate::context;
use crate::control::{RunControl, RunControlDecision};
use crate::error::{WorkflowError, WorkflowResult};
use crate::executor_output::{ensure_fanout_item_output_usable, ensure_output_usable};
use crate::fanout::{self, FanoutItem};
use crate::persistence;
use crate::policy::WorkflowPolicy;
use crate::request::fanout_item_request;
use crate::run::{ItemState, RunStatus, StageStatus, WorkflowRun};
use crate::runner::{StageRunOutput, StageRunRequest, WorkflowStageRunner};
use crate::source_context;
use crate::spec::{StageKind, StageSpec};
use crate::store::WorkflowStore;
use crate::work_unit_coverage::CoverageVerdict;
use crate::work_unit_gate;

#[path = "../executor_fanout/coordinated_failure.rs"]
mod coordinated_failure;
#[path = "../executor_fanout/coordinated_outcome.rs"]
mod coordinated_outcome;
#[path = "../executor_fanout/coordinated_success.rs"]
mod coordinated_success;
#[path = "../executor_fanout/coverage_gate.rs"]
mod coverage_gate;
#[path = "../executor_fanout/empty.rs"]
mod empty;
#[path = "../executor_fanout/failure_records.rs"]
mod failure_records;
#[path = "../executor_fanout/required_artifact_repair.rs"]
mod required_artifact_repair;
#[path = "../executor_fanout/targets.rs"]
mod targets;
use coordinated_outcome::record_coordinated_outcome;
use empty::empty_fanout_result;
use failure_records::{record_failure, record_output_failure};
use targets::{item_target_files, stage_max_agents};

struct ItemAcceptance {
    root: PathBuf,
    targets: Vec<String>,
    payload: serde_json::Value,
}

enum ItemRunStatus {
    Accepted,
    Blocked,
}

pub(crate) async fn run_fanout_with_runner(
    store: &WorkflowStore,
    policy: &WorkflowPolicy,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    runner: &dyn WorkflowStageRunner,
) -> WorkflowResult<StageRunOutput> {
    let items = context::fanout_items(store, run, stage)?;
    let item_kind = stage.effective_item_kind();
    let implementation_items = item_kind == StageKind::Implementation;
    if items.is_empty() {
        return empty_fanout_result(store, run, stage, implementation_items);
    }
    if implementation_items
        && let Some(output) =
            try_coordinated_implementation(store, policy, run, stage, runner, &items).await?
    {
        return Ok(output);
    }
    let width = serial_or_clamped_width(implementation_items, run, policy, stage, runner);
    let max_agents = stage_max_agents(policy, run, stage);
    if items.len() > max_agents {
        return Err(WorkflowError::PolicyDenied(format!(
            "fan-out item count {} exceeds max_agents {max_agents}",
            items.len()
        )));
    }
    let mut completed = items
        .iter()
        .filter(|item| fanout::accepted_item_cached(run, &item.id))
        .count();
    let mut failed = 0usize;
    let mut blocked = 0usize;
    let mut acceptances = BTreeMap::new();
    let mut requests = Vec::new();
    let pending_items = items
        .iter()
        .filter(|item| !fanout::accepted_item_cached(run, &item.id))
        .cloned()
        .collect::<Vec<_>>();
    for item in &pending_items {
        if implementation_items {
            match item_acceptance(store, run, stage, item) {
                Ok(binding) => {
                    acceptances.insert(item.id.clone(), binding);
                }
                Err(err) => {
                    record_failure(store, run, stage, item.id.clone(), err.to_string())?;
                    failed += 1;
                    continue;
                }
            }
        }
        let request = fanout_item_request(store, run, stage, item)?;
        persistence::record_prompt(store, &request)?;
        requests.push((item.id.clone(), request));
    }
    let (results, control_stop) = run_item_batches_with_control(
        store,
        run,
        stage,
        requests,
        runner,
        width,
        max_agents,
        stage.retry.max_attempts,
    )
    .await?;
    for (item_id, result) in results {
        match result {
            Ok(output) => {
                let result = match acceptances.remove(&item_id) {
                    Some(binding) => record_implementation_success(
                        store,
                        run,
                        stage,
                        item_id,
                        output,
                        binding,
                        policy.missing_unit_remediation_max_attempts,
                    ),
                    None => record_success(store, run, stage, item_id, output),
                };
                match result {
                    Ok(ItemRunStatus::Accepted) => completed += 1,
                    Ok(ItemRunStatus::Blocked) => blocked += 1,
                    Err(_) => failed += 1,
                }
            }
            Err(err) => {
                record_failure(store, run, stage, item_id, err.to_string())?;
                failed += 1;
            }
        }
    }
    if let Some(stop) = control_stop {
        return match stop {
            RunControlDecision::Continue => Ok(StageRunOutput::markdown(format!(
                "Fan-out stage `{}` completed {} item(s), blocked {} item(s), failed {} item(s), width {}.",
                stage.id, completed, blocked, failed, width
            ))),
            RunControlDecision::Paused { generation } => Err(WorkflowError::ControlPaused(
                format!("fan-out paused at generation {generation} before pending item launch"),
            )),
            RunControlDecision::Cancelled { generation } => Err(WorkflowError::ControlCancelled(
                format!("fan-out cancelled at generation {generation} before pending item launch"),
            )),
        };
    }
    if failed > 0 {
        let kind = if implementation_items {
            "implementation fan-out"
        } else {
            "fan-out"
        };
        return Err(WorkflowError::StageFailed(format!(
            "{failed} {kind} item(s) failed"
        )));
    }
    if blocked > 0 {
        let kind = if implementation_items {
            "implementation fan-out"
        } else {
            "fan-out"
        };
        return Err(WorkflowError::StageBlocked(format!(
            "{blocked} {kind} item(s) blocked with evidence"
        )));
    }
    if implementation_items {
        coverage_gate::enforce_stage(
            store,
            run,
            stage,
            &items,
            policy.missing_unit_remediation_max_attempts,
        )?;
    }
    Ok(StageRunOutput::markdown(format!(
        "Fan-out stage `{}` completed {} item(s), blocked {} item(s), failed {} item(s), width {}.",
        stage.id, completed, blocked, failed, width
    )))
}
