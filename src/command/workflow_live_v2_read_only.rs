async fn run_read_only_v2_fanout(
    task: &str,
    runtime: &WorkflowV2ScriptRuntime,
    execution: WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: &WorkflowV2ResultStore,
    store_for_control: &WorkflowStore,
    run_id: &str,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let items = fanout_items_for_call(&execution, v2_store)?;
    // Read-only branches (verification waves) need the project artifact root
    // too: without it verifiers fall back to repo-relative paths and cannot
    // resolve declared artifacts, which the reference tells them to check
    // absolutely. Write branches already get this stamp.
    let items = workflow_live_v2_write::stamp_project_artifact_policy(items, v2_store);
    let item_order = branch_item_order(&items);
    let (reused_outcomes, pending_items) =
        split_reusable_branch_outcomes(v2_store, &execution.call.id, items)?;
    let max_parallelism =
        client.read_only_fanout_parallelism(execution.call.options.max_parallelism);
    let branch_timeout_secs =
        read_only_branch_timeout_secs(&execution.call.id, &runtime.generated_config);
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig {
        max_parallelism,
        branch_timeout: None,
        ..WorkflowV2SchedulerConfig::default()
    });
    let task = task.to_string();
    let target_repository_root = runtime.target_repository_root.clone();
    let parent_call_id = execution.call.id.clone();
    let branch_parent_call_id = execution.call.id.clone();
    let branch_store = v2_store.clone();
    let branch_control_store = store_for_control.clone();
    let branch_run_id = run_id.to_string();
    let branch_event_store = store_for_control.clone();
    let branch_event_run_id = run_id.to_string();
    let run_report = if pending_items.is_empty() {
        WorkflowV2FanoutReport {
            outcomes: Vec::new(),
            max_parallelism,
            peak_parallelism: 0,
            cancelled: false,
        }
    } else {
        for item in &pending_items {
            emit_v2_branch_event(
                store_for_control,
                run_id,
                WorkflowEventKind::StageStarted,
                serde_json::json!({
                    "event": "branch_queued",
                    "call_id": execution.call.id,
                    "branch_id": item.id,
                    "timeout_secs": branch_timeout_secs,
                    "capacity": "waiting_for_v2_scheduler_and_subagent_capacity",
                }),
            );
        }
        scheduler
            .run_read_only_fanout_observed(
                pending_items,
                move |outcome| {
                    branch_store.save_branch_outcome(&branch_parent_call_id, outcome)?;
                    emit_v2_branch_event(
                        &branch_event_store,
                        &branch_event_run_id,
                        if matches!(
                            outcome.status,
                            WorkflowV2Status::Failed | WorkflowV2Status::Cancelled
                        ) {
                            WorkflowEventKind::StageFailed
                        } else {
                            WorkflowEventKind::StageCompleted
                        },
                        serde_json::json!({
                            "event": branch_event_label(outcome),
                            "call_id": branch_parent_call_id,
                            "branch_id": outcome.item_id,
                            "status": outcome.status,
                            "failure_kind": outcome.failure_kind,
                            "error": outcome.error,
                        }),
                    );
                    Ok(())
                },
                |branch| {
                    let adapter = adapter.clone();
                    let task = task.clone();
                    let parent_call_id = parent_call_id.clone();
                    let control_store = branch_control_store.clone();
                    let run_id = branch_run_id.clone();
                    let target_repository_root = target_repository_root.clone();
                    let branch_client = client.with_timeout_secs(Some(branch_timeout_secs));
                    async move {
                        poll_v2_run_control(&control_store, &run_id, &branch.id)?;
                        emit_v2_branch_event(
                            &control_store,
                            &run_id,
                            WorkflowEventKind::StageStarted,
                            serde_json::json!({
                                "event": "branch_started",
                                "call_id": parent_call_id,
                                "branch_id": branch.id,
                                "timeout_secs": branch_timeout_secs,
                                "capacity": "workflow_scheduler_admitted_subagent_executor_may_wait",
                            }),
                        );
                        let branch_execution = WorkflowV2CallExecution {
                            call: branch.call.clone(),
                            input: branch.input.clone(),
                            depends_on: vec![parent_call_id],
                        };
                        let result = match workflow_live_v2_manifest_scope::
                            manifest_scope_verification_result(&branch_execution.input)
                        {
                            Some(result) => result,
                            None => run_single_v2_agent_call(
                                &task,
                                target_repository_root.clone(),
                                &branch_execution,
                                &adapter,
                                &branch_client,
                                None,
                            )
                            .await?,
                        };
                        poll_v2_run_control(&control_store, &run_id, &branch.id)?;
                        Ok(result)
                    }
                },
            )
            .await?
    };
    let mut outcomes = reused_outcomes;
    outcomes.extend(run_report.outcomes);
    sort_branch_outcomes_by_order(&mut outcomes, &item_order);
    let report = WorkflowV2FanoutReport {
        outcomes,
        max_parallelism: run_report.max_parallelism,
        peak_parallelism: run_report.peak_parallelism,
        cancelled: run_report.cancelled,
    };
    let normalized = result_from_fanout_report(&execution.call, report);
    for outcome in &normalized.outcomes {
        v2_store.save_branch_outcome(&execution.call.id, outcome)?;
    }
    let branch_artifact_paths = normalized
        .outcomes
        .iter()
        .map(|outcome| {
            v2_store
                .branch_outcome_path(&execution.call.id, &outcome.item_id)
                .display()
                .to_string()
        })
        .collect::<Vec<_>>();
    let mut result = normalized.result;
    if let Some(object) = result.data.as_object_mut() {
        object.insert(
            "branch_artifact_paths".to_string(),
            serde_json::json!(branch_artifact_paths),
        );
    }
    Ok(result)
}

fn read_only_branch_timeout_secs(call_id: &str, config: &GeneratedWorkflowConfig) -> u64 {
    if call_id.starts_with("verification-wave-") || call_id.starts_with("review-verification-wave-")
    {
        return u64::from(config.verification_branch_timeout_secs);
    }
    u64::from(config.host_call_timeout_secs)
}

fn branch_event_label(outcome: &WorkflowV2BranchOutcome) -> &'static str {
    if outcome
        .error
        .as_deref()
        .is_some_and(|error| error.to_ascii_lowercase().contains("timed out"))
    {
        return "branch_timed_out";
    }
    if outcome.status == WorkflowV2Status::Cancelled {
        return "branch_cancelled";
    }
    if outcome.status == WorkflowV2Status::Failed {
        return "branch_failed";
    }
    "branch_finished"
}

fn emit_v2_branch_event(
    store: &WorkflowStore,
    run_id: &str,
    kind: WorkflowEventKind,
    detail: serde_json::Value,
) {
    let Ok(seq) = store.next_event_seq(run_id) else {
        return;
    };
    let _ = WorkflowEventLog::new(store.clone()).emit(run_id, seq, kind, detail);
}
