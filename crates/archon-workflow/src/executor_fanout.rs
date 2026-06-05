use std::collections::BTreeMap;

use serde_json::Value;

use crate::acceptance::{self, TargetFingerprints};
use crate::context;
use crate::error::{WorkflowError, WorkflowResult};
use crate::executor_output::ensure_output_usable;
use crate::fanout::{self, FanoutItem};
use crate::persistence;
use crate::policy::WorkflowPolicy;
use crate::request::fanout_item_request;
use crate::run::{StageStatus, WorkflowRun};
use crate::runner::{StageRunOutput, WorkflowStageRunner};
use crate::source_context;
use crate::spec::{ProviderTier, StageKind, StageSpec};
use crate::store::WorkflowStore;

struct ItemAcceptance {
    targets: Vec<String>,
    before: TargetFingerprints,
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
    let width = if implementation_items {
        1
    } else {
        fanout::runner_clamped_width(run, policy, stage, runner.max_concurrency())
    };
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
    let results = fanout::run_items_with_runner(
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
                    Some(binding) => {
                        record_implementation_success(store, run, stage, item_id, output, binding)
                    }
                    None => record_success(store, run, stage, item_id, output),
                };
                match result {
                    Ok(()) => completed += 1,
                    Err(_) => failed += 1,
                }
            }
            Err(err) => {
                record_failure(store, run, stage, item_id, err.to_string())?;
                failed += 1;
            }
        }
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
    Ok(StageRunOutput::markdown(format!(
        "Fan-out stage `{}` completed {} item(s), failed {} item(s), width {}.",
        stage.id, completed, failed, width
    )))
}

fn record_success(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    item_id: String,
    output: StageRunOutput,
) -> WorkflowResult<()> {
    if let Err(err) = ensure_output_usable(&output.body) {
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
    Ok(())
}

fn record_implementation_success(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    item_id: String,
    output: StageRunOutput,
    binding: ItemAcceptance,
) -> WorkflowResult<()> {
    if let Err(err) = ensure_output_usable(&output.body) {
        let error = err.to_string();
        record_output_failure(store, run, stage, item_id.clone(), output, error.clone())?;
        return Err(WorkflowError::StageFailed(format!("{item_id}: {error}")));
    }
    let root = source_context::effective_root(store, run);
    let after = acceptance::snapshot_targets(&root, &binding.targets);
    let outcome = acceptance::evaluate(
        &root,
        &binding.targets,
        &binding.before,
        &after,
        stage.verify_command.as_deref(),
    );
    let accepted = outcome.is_accepted();
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
            Ok(())
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

fn record_failure(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    item_id: String,
    error: String,
) -> WorkflowResult<()> {
    let body =
        format!("# Fan-out Item Failed\n\nitem: `{item_id}`\nstatus: failed\nerror: {error}\n");
    let artifact =
        persistence::write_attached_stage_artifact(store, run, stage, &item_id, "md", body, false)?;
    persistence::record_agent_output(
        store,
        &run.id,
        &stage.id,
        &item_id,
        None,
        Some(&artifact),
        false,
        Some(&error),
    )?;
    fanout::record_item(
        run,
        stage,
        item_id,
        StageStatus::Failed,
        Some(artifact),
        Some(error),
    );
    Ok(())
}

fn record_output_failure(
    store: &WorkflowStore,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    item_id: String,
    output: StageRunOutput,
    error: String,
) -> WorkflowResult<()> {
    let artifact = persistence::write_attached_stage_artifact(
        store,
        run,
        stage,
        &item_id,
        &output.extension,
        output.body.clone(),
        false,
    )?;
    persistence::record_agent_output(
        store,
        &run.id,
        &stage.id,
        &item_id,
        Some(&output),
        Some(&artifact),
        false,
        Some(&error),
    )?;
    fanout::record_item(
        run,
        stage,
        item_id,
        StageStatus::Failed,
        Some(artifact),
        Some(error),
    );
    Ok(())
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
    let root = source_context::effective_root(store, run);
    let before = acceptance::snapshot_targets(&root, &targets);
    Ok(ItemAcceptance { targets, before })
}

fn item_target_files(stage: &StageSpec, payload: &Value) -> Vec<String> {
    let mut targets = string_list(payload.get("target_files"))
        .into_iter()
        .chain(string_list(payload.get("expected_target_files")))
        .chain(string_list(payload.get("target_file")))
        .chain(string_list(payload.get("target_path")))
        .collect::<Vec<_>>();
    if targets.is_empty() {
        targets = stage.expected_target_files.clone();
    }
    targets.retain(|target| !target.trim().is_empty());
    targets
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(value)) => vec![value.clone()],
        _ => Vec::new(),
    }
}

fn stage_max_agents(policy: &WorkflowPolicy, run: &WorkflowRun, stage: &StageSpec) -> usize {
    let base = run.spec.max_agents.min(policy.max_agents_per_run);
    let effective = if stage.provider_tier == Some(ProviderTier::Local) {
        base.min(policy.local_provider_max_agents)
    } else {
        base
    };
    effective as usize
}
