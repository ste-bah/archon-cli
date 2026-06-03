use crate::context;
use crate::error::WorkflowResult;
use crate::fanout::FanoutItem;
use crate::run::WorkflowRun;
use crate::runner::StageRunRequest;
use crate::spec::{ProviderTier, StageSpec};
use crate::store::WorkflowStore;

pub(crate) fn stage_request(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<StageRunRequest> {
    let attempt = run
        .stages
        .get(&stage.id)
        .map(|state| state.attempt)
        .unwrap_or(1);
    Ok(StageRunRequest {
        run_id: run.id.clone(),
        stage_id: stage.id.clone(),
        stage_kind: stage.kind,
        agent: stage.agent.clone(),
        task: stage.task.clone().unwrap_or_else(|| run.spec.task.clone()),
        attempt,
        provider_tier: stage.provider_tier.unwrap_or(ProviderTier::Planner),
        depends_on: stage.depends_on.clone(),
        input: context::stage_input(store, run, stage)?,
    })
}

pub(crate) fn fanout_item_request(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
    item: &FanoutItem,
) -> WorkflowResult<StageRunRequest> {
    let mut request = stage_request(store, run, stage)?;
    request.stage_id = item.id.clone();
    request.input = context::fanout_input(store, run, stage, item)?;
    Ok(request)
}
