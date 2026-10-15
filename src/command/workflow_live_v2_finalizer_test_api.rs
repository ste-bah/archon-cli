use super::*;

pub(in crate::command::workflow_live::workflow_live_v2) async fn finalize_summary(
    store: &WorkflowStore,
    run_id: &str,
    run_kind: WorkflowRunKind,
    snapshot: Option<RunEndAcceptanceObserverSnapshotV1>,
    summary: &WorkflowV2ScriptSummary,
    v2_store: &WorkflowV2ResultStore,
    observer: Option<&dyn WorkflowRunEndObserver>,
    expected_generation: Option<u64>,
) -> WorkflowResult<()> {
    finalize_summary_with_limits(
        store,
        run_id,
        run_kind,
        snapshot,
        summary,
        v2_store,
        observer,
        expected_generation,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    )
    .await
}
