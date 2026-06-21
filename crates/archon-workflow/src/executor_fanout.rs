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

mod coordinated_failure;
mod coordinated_outcome;
mod coordinated_success;
mod coverage_gate;
mod empty;
mod failure_records;
mod required_artifact_repair;
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

type FanoutResult = (String, WorkflowResult<StageRunOutput>);

async fn run_item_batches_with_control(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    requests: Vec<(String, StageRunRequest)>,
    runner: &dyn WorkflowStageRunner,
    width: usize,
    max_agents: usize,
    max_attempts: u32,
) -> WorkflowResult<(Vec<FanoutResult>, Option<RunControlDecision>)> {
    if requests.len() > max_agents {
        return Err(WorkflowError::PolicyDenied(format!(
            "fan-out item count {} exceeds max_agents {max_agents}",
            requests.len()
        )));
    }
    let mut results = Vec::new();
    let mut idx = 0usize;
    while idx < requests.len() {
        match RunControl::new(store.clone(), run.id.clone()).checkpoint(run)? {
            RunControlDecision::Continue => {}
            decision @ RunControlDecision::Paused { .. } => {
                return Ok((results, Some(decision)));
            }
            decision @ RunControlDecision::Cancelled { .. } => {
                cancel_pending_items(run, stage, &requests[idx..]);
                return Ok((results, Some(decision)));
            }
        }
        let end = (idx + width.max(1)).min(requests.len());
        let chunk = requests[idx..end].to_vec();
        let chunk_results =
            fanout::run_items_with_runner(chunk, runner, width, max_agents, max_attempts).await?;
        results.extend(chunk_results);
        idx = end;
        match RunControl::new(store.clone(), run.id.clone()).checkpoint(run)? {
            RunControlDecision::Continue => {}
            decision @ RunControlDecision::Paused { .. } => {
                return Ok((results, Some(decision)));
            }
            decision @ RunControlDecision::Cancelled { .. } => {
                cancel_pending_items(run, stage, &requests[idx..]);
                return Ok((results, Some(decision)));
            }
        }
    }
    Ok((results, None))
}

fn cancel_pending_items(
    run: &mut WorkflowRun,
    stage: &StageSpec,
    pending: &[(String, StageRunRequest)],
) {
    run.status = RunStatus::Cancelled;
    for (item_id, _) in pending {
        run.items.insert(
            item_id.clone(),
            ItemState {
                id: item_id.clone(),
                stage_id: stage.id.clone(),
                status: StageStatus::Cancelled,
                artifact: None,
                error: Some("cancelled before launch".to_string()),
            },
        );
    }
    run.mark_updated();
}

fn serial_or_clamped_width(
    impl_items: bool,
    run: &WorkflowRun,
    policy: &WorkflowPolicy,
    stage: &StageSpec,
    runner: &dyn WorkflowStageRunner,
) -> usize {
    if impl_items {
        1
    } else {
        fanout::runner_clamped_width(run, policy, stage, runner.max_concurrency())
    }
}

async fn try_coordinated_implementation(
    store: &WorkflowStore,
    policy: &WorkflowPolicy,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    runner: &dyn WorkflowStageRunner,
    items: &[FanoutItem],
) -> WorkflowResult<Option<StageRunOutput>> {
    use crate::write_coordinator::coordinator::run_coordinated_implementation_fanout;
    use crate::write_coordinator::coordinator::{FanoutCtx, PlanInput};
    use crate::write_coordinator::resolve_write_coordinator_runtime;

    let cfg = policy.write_coordinator.clone();
    let Some(canonical) = run
        .spec
        .target_repository_root
        .as_deref()
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
    else {
        return Ok(None);
    };
    let runtime = resolve_write_coordinator_runtime(&canonical, &cfg);
    let plans_input: Vec<PlanInput> = items
        .iter()
        .map(|item| PlanInput {
            item: item.clone(),
            target_files: item_target_files(stage, &item.payload),
        })
        .collect();
    if plans_input.iter().any(|plan| {
        source_context::item_targets_need_serial_root(
            store,
            run,
            &plan.item.payload,
            &plan.target_files,
            &canonical,
        )
    }) {
        return Ok(None);
    }
    let run_root = store.run_dir(&run.id);
    let outcome = {
        let ctx = FanoutCtx {
            store,
            run,
            policy,
            stage,
            run_root,
            item_deps: BTreeMap::new(),
            verify_inputs: Vec::new(),
        };
        match run_coordinated_implementation_fanout(&ctx, plans_input, &runtime, &cfg, runner).await
        {
            Ok(outcome) => outcome,
            Err(err) => {
                let message = err.to_string();
                if message.contains("ControlPaused") {
                    let _ = RunControl::new(store.clone(), run.id.clone()).checkpoint(run)?;
                    return Err(WorkflowError::ControlPaused(message));
                }
                if message.contains("ControlCancelled") {
                    let _ = RunControl::new(store.clone(), run.id.clone()).checkpoint(run)?;
                    return Err(WorkflowError::ControlCancelled(message));
                }
                return Err(WorkflowError::StageFailed(message));
            }
        }
    };
    let seq_base = (run.stages.len() as u64 + 1) * 100_000;
    crate::events::write_coordination_events::emit_and_record(store, seq_base, &outcome);
    if let Some(_reason) = outcome.serial_fallback {
        return Ok(None);
    }
    record_coordinated_outcome(
        store,
        run,
        stage,
        &outcome,
        policy.missing_unit_remediation_max_attempts,
    )?;
    let applied = outcome
        .item_status
        .values()
        .filter(|s| matches!(s, crate::write_coordinator::ManifestStatus::Applied))
        .count();
    Ok(Some(StageRunOutput::markdown(format!(
        "Coordinated implementation fan-out `{}` applied {applied} item(s) across {} wave(s).",
        stage.id,
        outcome.waves.len()
    ))))
}

fn record_success(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    item_id: String,
    output: StageRunOutput,
) -> WorkflowResult<ItemRunStatus> {
    if let Err(err) = ensure_fanout_item_output_usable(stage, &output.body) {
        let error = err.to_string();
        record_output_failure(store, run, stage, item_id.clone(), output, error.clone())?;
        return Err(WorkflowError::StageFailed(format!("{item_id}: {error}")));
    }
    let artifact = persistence::write_attached_stage_artifact(
        store,
        run,
        stage,
        &item_id,
        &output.extension,
        output.body.clone(),
        true,
    )?;
    persistence::record_agent_output(
        store,
        &run.id,
        &stage.id,
        &item_id,
        Some(&output),
        Some(&artifact),
        true,
        None,
    )?;
    fanout::record_item(
        run,
        stage,
        item_id,
        StageStatus::Accepted,
        Some(artifact),
        None,
    );
    Ok(ItemRunStatus::Accepted)
}

fn record_implementation_success(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    item_id: String,
    output: StageRunOutput,
    binding: ItemAcceptance,
    max_remediation_attempts: u32,
) -> WorkflowResult<ItemRunStatus> {
    if required_artifact_repair::is_blocked_evidence(stage, &output.body) {
        required_artifact_repair::record_evidence(store, run, stage, item_id, output)?;
        return Ok(ItemRunStatus::Blocked);
    }
    if let Err(err) = ensure_output_usable(&output.body)
        && !required_artifact_repair::is_accepted_report_evidence(stage, &output.body)
    {
        let error = err.to_string();
        record_output_failure(store, run, stage, item_id.clone(), output, error.clone())?;
        return Err(WorkflowError::StageFailed(format!("{item_id}: {error}")));
    }
    let root = binding.root;
    let after = acceptance::snapshot_targets(&root, &binding.targets);
    let outcome = coverage_gate::evaluate_item(store, run, stage, &root, &binding.targets, &after)?;
    let coverage =
        work_unit_gate::evaluate_item_output(run, stage, &item_id, &binding.payload, &output.body);
    let coverage_accepted = coverage
        .as_ref()
        .is_none_or(|coverage| coverage.verdict == CoverageVerdict::Accepted);
    let accepted = outcome.is_accepted() && coverage_accepted;
    let artifact = persistence::write_attached_stage_artifact(
        store,
        run,
        stage,
        &item_id,
        &output.extension,
        output.body.clone(),
        accepted,
    )?;
    match outcome {
        acceptance::AcceptanceOutcome::Accepted => {
            if let Some(coverage) = coverage
                && coverage.verdict != CoverageVerdict::Accepted
            {
                let reason = work_unit_gate::error_summary(&coverage);
                let key = format!("__work_unit_coverage_{item_id}");
                work_unit_gate::write_named_coverage_artifact(
                    store, run, stage, &key, &coverage, false,
                )?;
                let remediation = crate::work_unit_remediation::write_missing_unit_items(
                    store,
                    run,
                    stage,
                    &coverage,
                    vec![crate::work_unit_remediation::source_from_payload(
                        &binding.payload,
                    )],
                    max_remediation_attempts,
                )?;
                persistence::record_agent_output(
                    store,
                    &run.id,
                    &stage.id,
                    &item_id,
                    Some(&output),
                    Some(&artifact),
                    false,
                    Some(&reason),
                )?;
                fanout::record_item(
                    run,
                    stage,
                    item_id.clone(),
                    StageStatus::Failed,
                    Some(artifact),
                    Some(reason.clone()),
                );
                if remediation.attempts_exhausted {
                    return Err(WorkflowError::StageBlocked(format!("{item_id}: {reason}")));
                }
                return Err(WorkflowError::StageFailed(format!("{item_id}: {reason}")));
            }
            persistence::record_agent_output(
                store,
                &run.id,
                &stage.id,
                &item_id,
                Some(&output),
                Some(&artifact),
                true,
                None,
            )?;
            fanout::record_item(
                run,
                stage,
                item_id,
                StageStatus::Accepted,
                Some(artifact),
                None,
            );
            Ok(ItemRunStatus::Accepted)
        }
        acceptance::AcceptanceOutcome::Rejected(reason) => {
            persistence::record_agent_output(
                store,
                &run.id,
                &stage.id,
                &item_id,
                Some(&output),
                Some(&artifact),
                false,
                Some(&reason),
            )?;
            fanout::record_item(
                run,
                stage,
                item_id.clone(),
                StageStatus::Failed,
                Some(artifact),
                Some(reason.clone()),
            );
            Err(WorkflowError::StageFailed(format!("{item_id}: {reason}")))
        }
    }
}

fn item_acceptance(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
    item: &FanoutItem,
) -> WorkflowResult<ItemAcceptance> {
    let targets = item_target_files(stage, &item.payload);
    if targets.is_empty() {
        return Err(WorkflowError::StageFailed(format!(
            "implementation fan-out item '{}' declares no target_files",
            item.id
        )));
    }
    let root = source_context::implementation_root_for_payload_targets(
        store,
        run,
        &item.payload,
        &targets,
    )?;
    Ok(ItemAcceptance {
        root,
        targets,
        payload: item.payload.clone(),
    })
}
