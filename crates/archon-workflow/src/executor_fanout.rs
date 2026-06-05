use crate::context;
use crate::error::{WorkflowError, WorkflowResult};
use crate::executor_output::ensure_output_usable;
use crate::fanout;
use crate::persistence;
use crate::policy::WorkflowPolicy;
use crate::request::fanout_item_request;
use crate::run::{StageStatus, WorkflowRun};
use crate::runner::{StageRunOutput, WorkflowStageRunner};
use crate::spec::{ProviderTier, StageSpec};
use crate::store::WorkflowStore;

pub(crate) async fn run_fanout_with_runner(
    store: &WorkflowStore,
    policy: &WorkflowPolicy,
    run: &mut WorkflowRun,
    stage: &StageSpec,
    runner: &dyn WorkflowStageRunner,
) -> WorkflowResult<StageRunOutput> {
    let items = context::fanout_items(store, run, stage)?;
    let width = fanout::runner_clamped_width(run, policy, stage, runner.max_concurrency());
    let max_agents = stage_max_agents(policy, run, stage);
    if items.len() > max_agents {
        return Err(WorkflowError::PolicyDenied(format!(
            "fan-out item count {} exceeds max_agents {max_agents}",
            items.len()
        )));
    }
    let requests = items
        .iter()
        .filter(|item| !fanout::accepted_item_cached(run, &item.id))
        .map(|item| {
            let request = fanout_item_request(store, run, stage, item)?;
            persistence::record_prompt(store, &request)?;
            Ok((item.id.clone(), request))
        })
        .collect::<WorkflowResult<Vec<_>>>()?;
    let mut completed = items.len().saturating_sub(requests.len());
    let mut failed = 0usize;
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
                record_success(store, run, stage, item_id, output)?;
                completed += 1;
            }
            Err(err) => {
                record_failure(store, run, stage, item_id, err.to_string())?;
                failed += 1;
            }
        }
    }
    if completed == 0 && failed > 0 {
        return Err(WorkflowError::StageFailed(format!(
            "all {failed} fan-out item(s) failed"
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
    ensure_output_usable(&output.body)
        .map_err(|err| WorkflowError::StageFailed(format!("{item_id}: {err}")))?;
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

fn stage_max_agents(policy: &WorkflowPolicy, run: &WorkflowRun, stage: &StageSpec) -> usize {
    let base = run.spec.max_agents.min(policy.max_agents_per_run);
    let effective = if stage.provider_tier == Some(ProviderTier::Local) {
        base.min(policy.local_provider_max_agents)
    } else {
        base
    };
    effective as usize
}
