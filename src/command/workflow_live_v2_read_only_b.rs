use super::*;

pub(super) async fn run_read_only_v2_fanout(
    task: &str,
    runtime: &WorkflowV2ScriptRuntime,
    execution: WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: &WorkflowV2ResultStore,
    store_for_control: &WorkflowStore,
    run_id: &str,
    task_universe: Option<&archon_workflow::task_universe::WorkflowV2TaskUniverse>,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let items = fanout_items_for_call(&execution, v2_store)?;
    // Read-only branches (verification waves) need the project artifact root
    // too: without it verifiers fall back to repo-relative paths and cannot
    // resolve declared artifacts, which the reference tells them to check
    // absolutely. Write branches already get this stamp.
    let items = workflow_live_v2_write::stamp_project_artifact_policy(items, v2_store);
    // Bind each branch to the contracts its task declared. The v3 authored
    // prelude builds its own verification items and never attaches one, so
    // without this the host verifier below has nothing to enforce.
    let items = stamp_declared_contracts_from_universe(items, task_universe);
    // Same asymmetry as contracts: write branches and the decomposed path bind
    // declared tools, the v3 authored path did not — so a verifier could be
    // asked to prove live tool invocations it had no way to make.
    let items = stamp_required_tools_from_universe(items, task_universe);
    // Capture each item's declared deliverable contracts (plus the artifact root
    // its paths resolve against) BEFORE the items are consumed by scheduling, so
    // the host can run the contract verifier itself instead of trusting the
    // branch's self-report. See enforce_declared_contracts.
    let declared_contracts = declared_contracts_by_item(&items);
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
    // Read-only branches (verification waves) must resolve project artifacts
    // absolutely: passing the store is what populates request.project_artifacts,
    // which is the typed field the prompt renders project_artifact_root from.
    let branch_artifact_store = v2_store.clone();
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
                    let artifact_store = branch_artifact_store.clone();
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
                                Some(&artifact_store),
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
    // Host-executed contract enforcement runs BEFORE aggregation so a demoted
    // branch also lowers the call's aggregate status; demoting afterwards would
    // leave an already-computed "accepted" result standing.
    workflow_live_v2_verification::enforce_declared_contracts(&mut outcomes, &declared_contracts)
        .await;
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

#[cfg(test)]
mod declared_contract_stamping_tests {
    use archon_workflow::task_universe::WorkflowV2TaskUniverse;

    fn universe() -> WorkflowV2TaskUniverse {
        serde_json::from_value(serde_json::json!({
            "schema_version": "v1",
            "source_roots": ["tasks"],
            "tasks": [{
                "canonical_task_id": "TASK-EX-001",
                "source_path": "tasks/TASK-EX-001.md",
                "required_tools": ["read_tool", "probe_tool"],
                "deliverable_contracts": [{
                    "kind": "record_series",
                    "artifact_path": ".archon/demo/coverage.json"
                }]
            }, {
                "canonical_task_id": "TASK-EX-002",
                "source_path": "tasks/TASK-EX-002.md"
            }]
        }))
        .expect("task universe")
    }

    fn item(id: &str, task_ids: serde_json::Value) -> archon_workflow::WorkflowV2FanoutItem {
        archon_workflow::WorkflowV2FanoutItem::read_only(
            id,
            "verifier",
            archon_workflow::WorkflowV2HostCall {
                id: "verification-wave".to_string(),
                method: archon_workflow::WorkflowV2HostMethod::Parallel,
                write_mode: None,
                options: archon_workflow::WorkflowV2HostOptions::default(),
            },
            serde_json::json!({
                "item": {"item_id": id, "canonical_task_ids": task_ids},
                "_workflow_project_artifact_policy": {"project_root": "/proj"}
            }),
        )
    }

    /// The v3 authored prelude builds its own verification item and never
    /// attaches a contract, so the host verifier had nothing to enforce and
    /// silently passed every branch. The universe is the authority.
    #[test]
    fn a_v3_verification_item_is_bound_to_its_tasks_declared_contracts() {
        let items = super::stamp_declared_contracts_from_universe(
            vec![item(
                "verify-task-ex-001",
                serde_json::json!(["TASK-EX-001"]),
            )],
            Some(&universe()),
        );
        let declared = super::declared_contracts_by_item(&items);
        let (root, contracts) = declared
            .get("verify-task-ex-001")
            .expect("contract must be bound");
        assert_eq!(root, "/proj");
        assert_eq!(contracts.len(), 1);
        assert_eq!(contracts[0]["artifact_path"], ".archon/demo/coverage.json");
    }

    /// A verifier asked to prove live tool invocations must be able to make
    /// them. Write branches and the decomposed path bound declared tools; the
    /// v3 authored path did not, so tasks whose acceptance requires tool calls
    /// were unverifiable by construction.
    #[test]
    fn a_v3_verification_item_is_granted_its_tasks_declared_tools() {
        let items = super::stamp_required_tools_from_universe(
            vec![item(
                "verify-task-ex-001",
                serde_json::json!(["TASK-EX-001"]),
            )],
            Some(&universe()),
        );
        let tools = items[0].input["item"]["required_tools"]
            .as_array()
            .expect("tools must be stamped");
        let names: Vec<&str> = tools.iter().filter_map(|t| t.as_str()).collect();
        assert_eq!(names, vec!["probe_tool", "read_tool"]);
    }

    /// Universe-sourced: a branch claiming a task that declares no tools gets
    /// none, so this cannot become a backdoor grant.
    #[test]
    fn a_task_declaring_no_tools_grants_none() {
        let items = super::stamp_required_tools_from_universe(
            vec![item(
                "verify-task-ex-002",
                serde_json::json!(["TASK-EX-002"]),
            )],
            Some(&universe()),
        );
        assert!(items[0].input["item"].get("required_tools").is_none());
    }

    #[test]
    fn a_branch_claiming_no_task_is_left_alone() {
        let items = super::stamp_declared_contracts_from_universe(
            vec![item("adversarial-review-map-0", serde_json::json!([]))],
            Some(&universe()),
        );
        assert!(items[0].input.get("deliverable_contracts").is_none());
        assert!(super::declared_contracts_by_item(&items).is_empty());
    }

    /// The decomposed path stamps a singular `deliverable_contract` per item;
    /// re-stamping would overwrite the contract that path deliberately chose.
    #[test]
    fn a_contract_already_stamped_by_the_decomposed_path_is_preserved() {
        let mut existing = item(
            "verify-TASK-EX-001-kind",
            serde_json::json!(["TASK-EX-001"]),
        );
        existing.input.as_object_mut().expect("object").insert(
            "deliverable_contract".to_string(),
            serde_json::json!({"kind": "chosen", "artifact_path": ".archon/chosen.json"}),
        );
        let items =
            super::stamp_declared_contracts_from_universe(vec![existing], Some(&universe()));
        assert!(items[0].input.get("deliverable_contracts").is_none());
        let declared = super::declared_contracts_by_item(&items);
        let (_, contracts) = declared.get("verify-TASK-EX-001-kind").expect("bound");
        assert_eq!(contracts.len(), 1);
        assert_eq!(contracts[0]["kind"], "chosen");
    }

    /// Without an artifact root the contract's relative paths cannot resolve,
    /// so there is nothing meaningful to verify against.
    #[test]
    fn a_branch_without_an_artifact_root_is_not_enforced() {
        let mut orphan = item("verify-task-ex-001", serde_json::json!(["TASK-EX-001"]));
        orphan
            .input
            .as_object_mut()
            .expect("object")
            .remove("_workflow_project_artifact_policy");
        let items = super::stamp_declared_contracts_from_universe(vec![orphan], Some(&universe()));
        assert!(items[0].input.get("deliverable_contracts").is_some());
        assert!(super::declared_contracts_by_item(&items).is_empty());
    }
}
