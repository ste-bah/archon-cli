use super::*;

pub(super) async fn run_serial_v2_write_fanout(
    task: &str,
    target_repository_root: Option<&str>,
    execution: &WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    dispatch: &dyn WorkflowAgentDispatch,
    v2_store: &WorkflowV2ResultStore,
    store_for_control: &archon_workflow::WorkflowStore,
    run_id: &str,
    branches: Vec<archon_workflow::WorkflowV2FanoutItem>,
    write_items: Vec<WorkflowV2WriteItem>,
    plan: WorkflowV2WritePlan,
    fallback_reason: Option<String>,
    reused_results: Vec<WorkflowV2Result>,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let mut branch_results = Vec::new();
    for branch in branches {
        let branch_id = branch.id.clone();
        let branch_role = branch.role.clone();
        let branch_input_hash = Some(branch.input_hash());
        poll_v2_run_control(store_for_control, run_id, &branch_id)?;
        let write_item = write_items
            .iter()
            .find(|item| item.id == branch_id)
            .ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "write item '{}' disappeared during validation",
                    branch_id
                ))
            })?;
        let mut branch_call = branch.call;
        branch_call.options.target_files = write_item.owned_targets.clone();
        branch_call.options.extra.insert(
            "target_ownership_scopes".to_string(),
            serde_json::to_value(&write_item.owned_scopes)?,
        );
        let branch_execution = WorkflowV2CallExecution {
            call: branch_call,
            input: branch.input,
            depends_on: vec![execution.call.id.clone()],
        };
        let mut result = match dispatch
            .run_call(
                task,
                target_repository_root.map(str::to_string),
                &branch_execution,
                &adapter,
                Some(v2_store),
                None,
            )
            .await
        {
            Ok(result) => result,
            Err(err) if is_recoverable_write_branch_timeout(&err.to_string()) => {
                write_branch_runtime_timeout_result(
                    &branch_id,
                    &branch_execution.input,
                    &err.to_string(),
                )
            }
            Err(err) if is_write_branch_validation_error(&err.to_string()) => {
                write_branch_validation_error_result(
                    &branch_id,
                    Some(&branch_execution.input),
                    &err.to_string(),
                )
            }
            Err(err) => return Err(err),
        };
        poll_v2_run_control(store_for_control, run_id, &branch_id)?;
        if let Err(err) =
            validate_changed_files_for_repository(write_item, &result, target_repository_root)
        {
            if is_write_branch_validation_error(&err.to_string()) {
                result = write_branch_validation_error_result(
                    &branch_id,
                    Some(&branch_execution.input),
                    &err.to_string(),
                );
            } else {
                return Err(WorkflowError::SpecInvalid(err.to_string()));
            }
        }
        if let Some(root) = target_repository_root
            && let Err(error) = verify_declared_artifacts_for_result(
                &branch_execution.input,
                &result,
                Path::new(root),
            )
        {
            result = write_branch_validation_error_result(
                &branch_id,
                Some(&branch_execution.input),
                &error,
            );
        }
        tag_branch_result(&mut result, &branch_id);
        normalize_write_branch_contract_result(&mut result);
        save_write_branch_outcome(
            v2_store,
            &execution.call.id,
            &branch_id,
            &branch_role,
            branch_input_hash,
            &result,
        )?;
        branch_results.push(result);
    }
    let mut all_results = reused_results;
    all_results.extend(branch_results);
    Ok(result_from_write_fanout(
        &execution.call,
        all_results,
        &plan,
        1,
        fallback_reason,
    ))
}
