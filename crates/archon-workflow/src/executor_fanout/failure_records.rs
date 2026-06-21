use crate::error::WorkflowResult;
use crate::fanout;
use crate::persistence;
use crate::run::{StageStatus, WorkflowRun};
use crate::runner::StageRunOutput;
use crate::spec::StageSpec;
use crate::store::WorkflowStore;

pub(super) fn record_failure(
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

pub(super) fn record_output_failure(
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
