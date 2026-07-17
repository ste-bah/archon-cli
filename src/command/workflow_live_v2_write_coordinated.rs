async fn run_coordinated_v2_write_fanout(
    task: &str,
    target_repository_root: Option<&str>,
    execution: &WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: &WorkflowV2ResultStore,
    store_for_control: &archon_workflow::WorkflowStore,
    run_id: &str,
    branches: Vec<archon_workflow::WorkflowV2FanoutItem>,
    plan: WorkflowV2WritePlan,
    reused_results: Vec<WorkflowV2Result>,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let write_items = write_items_for_branches(target_repository_root, &execution.call, &branches)?;
    let mut results = Vec::new();
    let mut peak_parallelism = 0usize;
    let max_parallelism = client.fanout_parallelism(execution.call.options.max_parallelism);
    for wave in &plan.waves {
        let semaphore = Arc::new(Semaphore::new(max_parallelism));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let jobs = wave.assignments.iter().map(|assignment| {
            let assignment = assignment.clone();
            let branch = branches
                .iter()
                .find(|branch| branch.id == assignment.item_id)
                .cloned();
            let adapter = adapter.clone();
            let control_store = store_for_control.clone();
            let run_id = run_id.to_string();
            let semaphore = semaphore.clone();
            let active = active.clone();
            let peak = peak.clone();
            async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|err| WorkflowError::StageFailed(err.to_string()))?;
                let branch = branch.ok_or_else(|| {
                    WorkflowError::SpecInvalid(format!(
                        "write plan referenced missing fanout item '{}'",
                        assignment.item_id
                    ))
                })?;
                poll_v2_run_control(&control_store, &run_id, &branch.id)?;
                let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
                record_write_peak(&peak, now_active);
                let mut branch_call = branch.call;
                branch_call.options.target_files = assignment.owned_targets.clone();
                branch_call.options.extra.insert(
                    "target_ownership_scopes".to_string(),
                    serde_json::to_value(&assignment.owned_scopes)?,
                );
                let branch_execution = WorkflowV2CallExecution {
                    call: branch_call,
                    input: branch.input,
                    depends_on: vec![execution.call.id.clone()],
                };
                let result = run_single_v2_agent_call(
                    task,
                    target_repository_root.map(str::to_string),
                    &branch_execution,
                    &adapter,
                    client,
                    Some(v2_store),
                )
                .await;
                active.fetch_sub(1, Ordering::SeqCst);
                let result = match result {
                    Ok(result) => result,
                    Err(err) if is_recoverable_write_branch_timeout(&err.to_string()) => {
                        write_branch_runtime_timeout_result(
                            &assignment.item_id,
                            &branch_execution.input,
                            &err.to_string(),
                        )
                    }
                    Err(err) => return Err(err),
                };
                poll_v2_run_control(&control_store, &run_id, &assignment.item_id)?;
                Ok::<WorkflowV2Result, WorkflowError>(result)
            }
        });
        let wave_results = futures_util::future::join_all(jobs).await;
        peak_parallelism = peak_parallelism.max(peak.load(Ordering::SeqCst));
        for (assignment, result) in wave.assignments.iter().zip(wave_results) {
            let mut result = match result {
                Ok(result) => result,
                Err(err) if is_write_branch_validation_error(&err.to_string()) => {
                    let input = branch_input_for_assignment(&branches, &assignment.item_id);
                    write_branch_validation_error_result(
                        &assignment.item_id,
                        input,
                        &err.to_string(),
                    )
                }
                Err(err) => return Err(err),
            };
            let write_item = write_items
                .iter()
                .find(|item| item.id == assignment.item_id)
                .ok_or_else(|| {
                    WorkflowError::SpecInvalid(format!(
                        "write item '{}' disappeared during validation",
                        assignment.item_id
                    ))
                })?;
            if let Err(err) =
                validate_changed_files_for_repository(write_item, &result, target_repository_root)
            {
                if is_write_branch_validation_error(&err.to_string()) {
                    let input = branch_input_for_assignment(&branches, &assignment.item_id);
                    result = write_branch_validation_error_result(
                        &assignment.item_id,
                        input,
                        &err.to_string(),
                    );
                } else {
                    return Err(WorkflowError::SpecInvalid(err.to_string()));
                }
            }
            if let Some(root) = target_repository_root
                && let Some(input) = branch_input_for_assignment(&branches, &assignment.item_id)
                && let Err(error) =
                    verify_declared_artifacts_for_result(input, &result, Path::new(root))
            {
                result = write_branch_validation_error_result(
                    &assignment.item_id,
                    Some(input),
                    &error,
                );
            }
            let role = branches
                .iter()
                .find(|branch| branch.id == assignment.item_id)
                .map(|branch| branch.role.as_str())
                .unwrap_or("coder");
            let item_input_hash = branches
                .iter()
                .find(|branch| branch.id == assignment.item_id)
                .map(|branch| branch.input_hash());
            tag_branch_result(&mut result, &assignment.item_id);
            normalize_write_branch_contract_result(&mut result);
            save_write_branch_outcome(
                v2_store,
                &execution.call.id,
                &assignment.item_id,
                role,
                item_input_hash,
                &result,
            )?;
            results.push(result);
        }
    }
    let mut all_results = reused_results;
    all_results.extend(results);
    Ok(result_from_write_fanout(
        &execution.call,
        all_results,
        &plan,
        peak_parallelism,
        None,
    ))
}

fn branch_input_for_assignment<'a>(
    branches: &'a [archon_workflow::WorkflowV2FanoutItem],
    item_id: &str,
) -> Option<&'a serde_json::Value> {
    branches
        .iter()
        .find(|branch| branch.id == item_id)
        .map(|branch| &branch.input)
}
