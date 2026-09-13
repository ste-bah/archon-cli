//! A direct write is a one-item wave, not permission to edit canonical source.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    task: &str, runtime: &WorkflowV2ScriptRuntime, execution: WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter, client: &LiveV2AgentClient, v2: &WorkflowV2ResultStore,
    store: &WorkflowStore, run_id: &str, workspace_boundary_supported: bool,
    universe: Option<&WorkflowV2TaskUniverse>, graph: Option<&archon_workflow::WorkflowV2SourceTaskGraph>,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let resolved = execution_with_resolved_source(&execution, v2)?;
    let mut input = resolved.input.clone();
    if input.get("item").is_none() {
        let item = input.get("source_data").cloned().unwrap_or_else(|| input.clone());
        input["item"] = item;
    }
    let branch = archon_workflow::WorkflowV2FanoutItem::read_only(
        format!("{}-direct", execution.call.id),
        execution.call.options.role.clone().unwrap_or_else(|| "coder".into()),
        WorkflowV2HostCall { id: format!("{}-direct", execution.call.id), ..resolved.call.clone() }, input,
    );
    let result = run_write_capable_v2_fanout(task, runtime.target_repository_root.as_deref(),
        resolved, adapter,
        &super::super::live_agent_dispatch::LiveAgentDispatch::new(client.clone())
            .with_generated_config(&runtime.generated_config),
        v2, store, run_id, workspace_boundary_supported, vec![branch], universe, graph).await?;
    // Keep the public single-call result shape; wave-level failure still wins.
    if let Some(item) = result.data.get("items").and_then(serde_json::Value::as_array).and_then(|items| items.first()) {
        let mut single: WorkflowV2Result = serde_json::from_value(item.clone())?;
        if !matches!(result.status, WorkflowV2Status::Accepted | WorkflowV2Status::Noop) {
            single.status = result.status;
            single.residual_gaps.extend(result.residual_gaps);
        }
        return Ok(single);
    }
    Ok(result)
}
