//! Batch O2 (CUT-11b): the live host runs the regression check before it
//! answers a residual pass's slot checkpoint (pass 2 on), so that pass can
//! route each regression to its owner
//! (`archon_workflow::v2::verification::regression_slot`). Only an executed
//! checkpoint gets here; a replayed one reads the record its first
//! execution wrote, so a recorded pass's plan never moves.

use super::*;

/// Run the check when `execution` is a residual pass's slot; a no-op for
/// every other call and for a run with no target repository. A check it
/// cannot record fails the call (m2): never a pass planned without it.
pub(super) async fn prepare(
    runtime: &WorkflowV2ScriptRuntime,
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    client: &LiveV2AgentClient,
) -> archon_workflow::WorkflowResult<()> {
    let Some(root) = runtime.target_repository_root.as_deref() else {
        return Ok(());
    };
    if archon_workflow::v2::script::residual_plan::slot_pass(&execution.call)
        .is_none_or(|pass| pass < 2)
    {
        return Ok(());
    }
    let dispatch = super::super::live_agent_dispatch::LiveAgentDispatch::new(client.clone())
        .with_generated_config(&runtime.generated_config);
    archon_workflow::v2::verification::regression_slot::prepare_slot(
        v2_store,
        &dispatch,
        task_universe,
        std::path::Path::new(root),
        &execution.call,
    )
    .await
}
