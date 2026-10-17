use anyhow::Result;
use archon_workflow::{RunStatus, WorkflowStore};

pub(crate) fn accepted_reverify(
    store: &WorkflowStore,
    run_id: &str,
    status: RunStatus,
) -> Result<Option<String>> {
    if status != RunStatus::Paused {
        return Ok(None);
    }
    let latest = crate::command::workflow_decompose_latest_report::latest_report(store, run_id)?;
    crate::command::workflow::workflow_decompose_reverify::eligibility::recover_accepted_reverify_before_resume(
        store,
        run_id,
        latest.as_ref(),
    )
}
