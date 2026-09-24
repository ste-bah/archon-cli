use super::*;

#[path = "workflow_live_v2_read_only_retry.rs"]
mod retry;
use retry::HostReask;

use archon_workflow::v2::branch_stamping::{
    declared_contracts_by_item, stamp_declared_contracts_from_universe,
    stamp_required_tools_from_universe,
};

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
    let mut items = fanout_items_for_call(&execution, v2_store)?;
    // Issue-70: a focused verifier runs its filter at the checkout's CURRENT
    // head, so its baseline is established there too — in the tree it will
    // `cd` into — before anything is dispatched. Red tests other tasks'
    // commits caused are routed to those tasks and exempted here, instead of
    // demoting this task's verdict for a failure it cannot fix. A run with
    // no target repository keeps the implementation-wave stamp.
    let mut judged_commit = None;
    if let Some(root) = runtime.target_repository_root.as_deref() {
        let dispatch = super::live_agent_dispatch::LiveAgentDispatch::new(client.clone())
            .with_generated_config(&runtime.generated_config);
        judged_commit = archon_workflow::v2::verification::establish_verification_baseline(
            &archon_workflow::v2::verification::VerificationBaselineContext {
                store: v2_store,
                dispatch: &dispatch,
                universe: task_universe,
                call_id: &execution.call.id,
                repository_root: std::path::Path::new(root),
                parallelism: client
                    .read_only_fanout_parallelism(execution.call.options.max_parallelism),
            },
            &mut items,
        )
        .await;
    }
    // Read-only branches (verification waves) need the project artifact root
    // too: without it verifiers fall back to repo-relative paths and cannot
    // resolve declared artifacts, which the reference tells them to check
    // absolutely. Write branches already get this stamp.
    let items = archon_workflow::v2::write::stamp_project_artifact_policy(items, v2_store);
    // Bind each branch to the contracts its task declared. The v3 authored
    // prelude builds its own verification items and never attaches one, so
    // without this the host verifier below has nothing to enforce.
    let items = stamp_declared_contracts_from_universe(items, task_universe);
    // Same asymmetry as contracts: write branches and the decomposed path bind
    // declared tools, the v3 authored path did not — so a verifier could be
    // asked to prove live tool invocations it had no way to make.
    let items = stamp_required_tools_from_universe(items, task_universe);
    // Issue-85: and who declares which path. A verifier is never given the
    // task universe, so it cannot tell a defect the task owns from one no
    // task owns — and withholding acceptance over the latter only loops,
    // because no branch may ever write it. The host answers that here and
    // stamps the conclusion; the universe itself never travels.
    let items = archon_workflow::v2::verification::stamp_path_ownership_from_universe(
        items,
        task_universe,
        runtime
            .target_repository_root
            .as_deref()
            .map(std::path::Path::new),
    );
    // Capture each item's declared deliverable contracts (plus the roots its
    // paths resolve against: project artifact root, then the target repository
    // root) BEFORE the items are consumed by scheduling, so the host can run
    // the contract verifier itself instead of trusting the branch's
    // self-report. See enforce_declared_contracts.
    let declared_contracts =
        declared_contracts_by_item(&items, runtime.target_repository_root.as_deref());
    // Obs-31: each verification item's base-commit test lists, stamped by
    // the item builder and re-stamped at the verification base above; held
    // here so the host can re-read the verifier's own report against them
    // after it returns (enforce_baseline_tests).
    let baseline_by_item = archon_workflow::v2::verification::baseline_by_item(&items);
    // Issue-81: and each branch's declared writable scope, for the same
    // reason at the same moment — a branch outcome carries no scope of its
    // own, and the host needs it to tell a finding the task could fix from
    // one naming a path no task in the universe declares at all.
    let scope_by_item = archon_workflow::v2::verification::scope_by_item(&items);
    let item_order = branch_item_order(&items);
    // Cargo-running branches share one serial scheduling role; everything else
    // runs at the wave's configured width. This replaces the wave-level
    // maxParallelism=1 pin that serialized entire verification waves for one
    // cargo item. Retagging changes only scheduling identity, never the input
    // (input hashes drive outcome reuse below).
    let items = archon_workflow::v2::lifecycle_policy::cargo_serial::tag_cargo_serial_roles(items);
    let (reused_outcomes, pending_items) =
        split_reusable_branch_outcomes(v2_store, &execution.call.id, items)?;
    let max_parallelism =
        client.read_only_fanout_parallelism(execution.call.options.max_parallelism);
    let (branch_timeout_secs, branch_timeout_source) =
        read_only_branch_timeout_secs(&execution.call.id, &runtime.generated_config);
    // A review map's failed branch is re-asked once: its task otherwise has no
    // verdict. Known by the contract the call declares, never by its name.
    let review_map = archon_workflow::v2::review_findings::is_review_map_call(&execution);
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig {
        max_parallelism,
        role_limits: archon_workflow::v2::lifecycle_policy::cargo_serial::cargo_serial_role_limits(
        ),
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
                    let branch_client =
                        client.with_timeout_secs(Some(branch_timeout_secs), branch_timeout_source);
                    let artifact_store = branch_artifact_store.clone();
                    let judged_commit = judged_commit.clone();
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
                        let result = match archon_workflow::v2::manifest_scope::
                            manifest_scope_verification_result(&branch_execution.input)
                        {
                            Some(result) => result,
                            None => {
                                let on_reask = |reason: HostReask, first: &str| {
                                    emit_v2_branch_event(
                                        &control_store,
                                        &run_id,
                                        WorkflowEventKind::StageStarted,
                                        serde_json::json!({
                                            "event": "branch_reasked",
                                            "call_id": branch_execution.depends_on.first(),
                                            "branch_id": branch.id,
                                            "reason": reason.label(),
                                            "attempt": 2,
                                            "first_error": first,
                                        }),
                                    );
                                };
                                run_read_only_call_with_retry(
                                    &task,
                                    &target_repository_root,
                                    &branch_execution,
                                    &adapter,
                                    &branch_client,
                                    &artifact_store,
                                    review_map,
                                    &on_reask,
                                )
                                .await?
                            }
                        };
                        poll_v2_run_control(&control_store, &run_id, &branch.id)?;
                        // The commit this verifier judged, on its own record
                        // (Issue-104): the baseline store is re-stamped at the
                        // current head on every resume.
                        let mut result = result;
                        archon_workflow::repository_audit::discharge::stamp_judged_commit(
                            &mut result,
                            judged_commit.as_deref(),
                        );
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
    archon_workflow::v2::verification::enforce_declared_contracts(
        &mut outcomes,
        &declared_contracts,
    )
    .await;
    archon_workflow::v2::verification::enforce_baseline_tests(&mut outcomes, &baseline_by_item);
    // A finding no branch can act on is recorded, never dispatched: without
    // this it retains as an ordinary gap and the lifecycle keeps sending a
    // writer at a path the write guard is right to refuse.
    if let Some(root) = runtime.target_repository_root.as_deref() {
        archon_workflow::v2::verification::flag_unowned_path_gaps(
            &mut outcomes,
            &scope_by_item,
            task_universe,
            std::path::Path::new(root),
        );
    }
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

/// The read-only twin of the write path's transport re-ask
/// (`worktree_branch_a`), plus the host's one re-ask: a dropped provider
/// connection is not a verdict on the work, so the branch is re-asked instead
/// of permanently failed; an inactivity cut, or any failure of a review map
/// branch, is re-asked exactly once. See `retry` for the budgets.
#[allow(clippy::too_many_arguments)]
async fn run_read_only_call_with_retry(
    task: &str,
    target_repository_root: &Option<String>,
    branch_execution: &WorkflowV2CallExecution,
    adapter: &WorkflowV2AgentAdapter,
    branch_client: &LiveV2AgentClient,
    artifact_store: &WorkflowV2ResultStore,
    review_map: bool,
    on_reask: &(dyn Fn(HostReask, &str) + Sync),
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    retry::with_host_retry(review_map, on_reask, || {
        run_single_v2_agent_call(
            task,
            target_repository_root.clone(),
            branch_execution,
            adapter,
            branch_client,
            Some(artifact_store),
            None,
            false,
        )
    })
    .await
}

/// The branch timeout and the name of the setting it came from.
fn read_only_branch_timeout_secs(
    call_id: &str,
    config: &GeneratedWorkflowConfig,
) -> (u64, &'static str) {
    if call_id.starts_with("verification-wave-") || call_id.starts_with("review-verification-wave-")
    {
        return (
            u64::from(config.verification_branch_timeout_secs),
            "verification_branch_timeout_secs",
        );
    }
    (
        u64::from(config.host_call_timeout_secs),
        "host_call_timeout_secs",
    )
}

fn branch_event_label(outcome: &WorkflowV2BranchOutcome) -> &'static str {
    // First: an inactivity cut also travels inside the host-cut wrapper, and
    // the record must name the bound that fired.
    if outcome
        .error
        .as_deref()
        .is_some_and(archon_workflow::error::is_inactivity_timeout_text)
    {
        return "branch_inactive";
    }
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
