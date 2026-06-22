use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

use anyhow::Result;
use archon_pipeline::runner::LlmClient;
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_workflow::{
    LifecycleAction, LifecycleController, ProviderTier, RunStatus, WorkflowBundle,
    WorkflowBundleOrigin, WorkflowError, WorkflowRun, WorkflowSpec, WorkflowStore,
    WorkflowV2AgentAdapter, WorkflowV2BranchOutcome, WorkflowV2CallExecution, WorkflowV2FanoutItem,
    WorkflowV2FanoutReport, WorkflowV2HarnessValidator, WorkflowV2HostMethod, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Scheduler, WorkflowV2SchedulerConfig, WorkflowV2Status,
};

#[path = "workflow_live_v2_aggregate.rs"]
mod workflow_live_v2_aggregate;
#[path = "workflow_live_v2_data.rs"]
mod workflow_live_v2_data;
#[cfg(test)]
#[path = "workflow_live_v2_data_tests.rs"]
mod workflow_live_v2_data_tests;

use workflow_live_v2_data::{
    execution_with_resolved_source, fanout_items_for_call, result_from_fanout_report,
    v2_agent_request,
};
#[path = "workflow_live_v2_client.rs"]
mod workflow_live_v2_client;

use workflow_live_v2_client::LiveV2AgentClient;
#[path = "workflow_live_v2_contracts.rs"]
mod workflow_live_v2_contracts;

#[path = "workflow_live_v2_write.rs"]
mod workflow_live_v2_write;

use workflow_live_v2_write::run_write_capable_v2_fanout;
#[path = "workflow_live_v2_state.rs"]
mod workflow_live_v2_state;

use workflow_live_v2_state::{poll_v2_run_control, sync_v2_summary_to_run};
#[path = "workflow_live_v2_script.rs"]
mod workflow_live_v2_script;

use workflow_live_v2_script::WorkflowV2ScriptRunner;

use super::LiveApprovalMode;
use super::workflow_live_approval::{LiveApprovalOutcome, gate_live_approval};
use super::workflow_live_compat::target_repository_root_from_task;
use super::workflow_live_planner::LivePlan;
use super::workflow_live_v2_host::execute_local_host_call;

pub(super) async fn run_generated_v2_workflow(
    cwd: &Path,
    store: &WorkflowStore,
    plan: LivePlan,
    task: String,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
    agent_names: Vec<String>,
    approval_mode: LiveApprovalMode,
    workspace_boundary_supported: bool,
) -> Result<String> {
    run_v2_workflow_with_origin(
        cwd,
        store,
        plan,
        task,
        llm,
        tui_tx,
        agent_names,
        approval_mode,
        workspace_boundary_supported,
        WorkflowBundleOrigin::GeneratedHarness,
    )
    .await
}

pub(super) async fn run_saved_v2_workflow(
    cwd: &Path,
    store: &WorkflowStore,
    plan: LivePlan,
    task: String,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
    agent_names: Vec<String>,
    approval_mode: LiveApprovalMode,
    workspace_boundary_supported: bool,
) -> Result<String> {
    run_v2_workflow_with_origin(
        cwd,
        store,
        plan,
        task,
        llm,
        tui_tx,
        agent_names,
        approval_mode,
        workspace_boundary_supported,
        WorkflowBundleOrigin::SavedCommand,
    )
    .await
}

async fn run_v2_workflow_with_origin(
    cwd: &Path,
    store: &WorkflowStore,
    plan: LivePlan,
    task: String,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
    agent_names: Vec<String>,
    approval_mode: LiveApprovalMode,
    workspace_boundary_supported: bool,
    origin: WorkflowBundleOrigin,
) -> Result<String> {
    let run = store.create_run(plan.spec.clone())?;
    WorkflowBundle::create_for_run(store, &run, &plan.harness_source, origin)?;
    let run = match gate_live_approval(cwd, store, run, approval_mode, &tui_tx)? {
        LiveApprovalOutcome::Proceed { run, note } => {
            let _ = tui_tx.send(TuiEvent::TextDelta(note.clone()));
            run
        }
        LiveApprovalOutcome::Pending(message) | LiveApprovalOutcome::Denied(message) => {
            return Ok(message);
        }
    };
    execute_generated_v2_run(
        store,
        run,
        plan,
        task,
        llm,
        tui_tx,
        agent_names,
        workspace_boundary_supported,
    )
    .await
}

pub(super) async fn resume_generated_v2_workflow(
    cwd: &Path,
    store: &WorkflowStore,
    run_id: &str,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
    agent_names: Vec<String>,
    approval_mode: LiveApprovalMode,
    workspace_boundary_supported: bool,
) -> Result<Option<String>> {
    let run = store.load_state(run_id)?;
    let Some(plan) = live_plan_from_generated_bundle(store, &run)? else {
        return Ok(None);
    };
    match run.status {
        RunStatus::Completed => {
            return Ok(Some(format!(
                "Workflow {} is already completed; start a new workflow run for new work.\n",
                run.id
            )));
        }
        RunStatus::Cancelled => {
            return Ok(Some(format!(
                "Workflow {} is cancelled and cannot be resumed; start a new workflow run.\n",
                run.id
            )));
        }
        _ => {}
    }
    let run = match gate_live_approval(cwd, store, run, approval_mode, &tui_tx)? {
        LiveApprovalOutcome::Proceed { run, note } => {
            let _ = tui_tx.send(TuiEvent::TextDelta(note.clone()));
            if matches!(run.status, RunStatus::Paused) {
                LifecycleController::new(store.clone()).apply(&run.id, LifecycleAction::Resume)?
            } else {
                run
            }
        }
        LiveApprovalOutcome::Pending(message) | LiveApprovalOutcome::Denied(message) => {
            return Ok(Some(message));
        }
    };
    let task = run.spec.task.clone();
    execute_generated_v2_run(
        store,
        run,
        plan,
        task,
        llm,
        tui_tx,
        agent_names,
        workspace_boundary_supported,
    )
    .await
    .map(Some)
}

fn live_plan_from_generated_bundle(
    store: &WorkflowStore,
    run: &WorkflowRun,
) -> Result<Option<LivePlan>> {
    let manifest = match WorkflowBundle::verify(store, &run.id) {
        Ok(manifest) => manifest,
        Err(_) => return Ok(None),
    };
    if !matches!(
        manifest.origin,
        WorkflowBundleOrigin::GeneratedHarness | WorkflowBundleOrigin::SavedCommand
    ) {
        return Ok(None);
    }
    let harness_path = store.run_dir(&run.id).join("workflow.js");
    let harness_source = fs::read_to_string(&harness_path).map_err(|err| WorkflowError::Io {
        path: harness_path.clone(),
        source: err,
    })?;
    let plan = WorkflowV2HarnessValidator
        .validate(&harness_source)
        .map_err(|err| WorkflowError::SpecInvalid(err.to_string()))?;
    Ok(Some(LivePlan {
        spec: run.spec.clone(),
        harness_source: harness_source.trim().to_string(),
        calls: plan.calls,
    }))
}

async fn execute_generated_v2_run(
    store: &WorkflowStore,
    run: WorkflowRun,
    plan: LivePlan,
    task: String,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
    agent_names: Vec<String>,
    workspace_boundary_supported: bool,
) -> Result<String> {
    let adapter = WorkflowV2AgentAdapter::new();
    let mut spec = plan.spec.clone();
    if spec
        .target_repository_root
        .as_deref()
        .is_none_or(|root| root.trim().is_empty())
    {
        spec.target_repository_root = target_repository_root_from_task(&task);
    }
    let client = LiveV2AgentClient::new(
        llm,
        tui_tx.clone(),
        agent_names,
        run.id.clone(),
        spec.target_repository_root.clone(),
    );
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let runner = WorkflowV2ScriptRunner::new(
        task,
        spec,
        adapter,
        client,
        v2_store.clone(),
        store.clone(),
        run.id.clone(),
        workspace_boundary_supported,
    );
    let summary = match runner.run(&plan.harness_source).await {
        Ok(summary) => summary,
        Err(WorkflowError::ControlPaused(message)) => {
            return Ok(format!(
                "Workflow paused: {}\n{}\nResume with: /workflow resume --live {}\n",
                run.id, message, run.id
            ));
        }
        Err(WorkflowError::ControlCancelled(message)) => {
            return Ok(format!("Workflow cancelled: {}\n{}\n", run.id, message));
        }
        Err(err) => return Err(err.into()),
    };

    sync_v2_summary_to_run(store, &run.id, &summary.calls, &v2_store, summary.status)?;
    let mut output = format!(
        "Workflow V2 complete: {} (status {:?}, completed {}, executed {}, reused {})\n",
        run.id, summary.status, summary.completed, summary.executed, summary.reused
    );
    output.push_str(&format!(
        "harness: {}\nv2_results: {}\n",
        store.run_dir(&run.id).join("workflow.js").display(),
        v2_store.root().display()
    ));
    Ok(output)
}

pub(super) fn source_call_ids(source: &str) -> Vec<String> {
    let trimmed = source.trim();
    if trimmed.starts_with('{') {
        return Vec::new();
    }
    let body = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(trimmed);
    body.split(',')
        .filter_map(|part| {
            let head = part
                .trim()
                .split_once('.')
                .map(|(head, _)| head)
                .unwrap_or(part);
            let id = head.trim().trim_matches(|ch| ch == '"' || ch == '\'');
            if id.is_empty() || id.starts_with('{') {
                None
            } else {
                Some(id.to_string())
            }
        })
        .collect()
}

async fn execute_v2_live_call(
    task: &str,
    spec: &WorkflowSpec,
    execution: WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: &WorkflowV2ResultStore,
    store_for_control: &WorkflowStore,
    run_id: &str,
    workspace_boundary_supported: bool,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    if matches!(
        execution.call.method,
        WorkflowV2HostMethod::Checkpoint
            | WorkflowV2HostMethod::SaveArtifact
            | WorkflowV2HostMethod::RequireArtifact
            | WorkflowV2HostMethod::FinalReport
            | WorkflowV2HostMethod::QualityGate
            | WorkflowV2HostMethod::HumanGate
    ) {
        let local_execution = if should_resolve_local_source(&execution) {
            execution_with_resolved_source(&execution, v2_store)?
        } else {
            execution.clone()
        };
        if let Some(result) = execute_local_host_call(&local_execution, v2_store)? {
            return Ok(result);
        }
    }
    match execution.call.method {
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel
            if execution.call.write_mode.is_none() =>
        {
            run_read_only_v2_fanout(
                task,
                spec,
                execution,
                adapter,
                client,
                v2_store,
                store_for_control,
                run_id,
            )
            .await
        }
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel => {
            run_write_capable_v2_fanout(
                task,
                spec,
                execution,
                adapter,
                client,
                v2_store,
                store_for_control,
                run_id,
                workspace_boundary_supported,
            )
            .await
        }
        _ => {
            run_single_v2_agent_call(task, spec, &execution, &adapter, client, Some(v2_store)).await
        }
    }
}

fn should_resolve_local_source(execution: &WorkflowV2CallExecution) -> bool {
    execution
        .call
        .options
        .source
        .as_deref()
        .is_some_and(|source| !source.trim_start().starts_with('{'))
}

async fn run_single_v2_agent_call(
    task: &str,
    spec: &WorkflowSpec,
    execution: &WorkflowV2CallExecution,
    adapter: &WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: Option<&WorkflowV2ResultStore>,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    run_single_v2_agent_call_in_repository(task, spec, execution, adapter, client, v2_store, None)
        .await
}

async fn run_single_v2_agent_call_in_repository(
    task: &str,
    spec: &WorkflowSpec,
    execution: &WorkflowV2CallExecution,
    adapter: &WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: Option<&WorkflowV2ResultStore>,
    repository_root_override: Option<String>,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let execution = match v2_store {
        Some(store) => execution_with_resolved_source(execution, store)?,
        None => execution.clone(),
    };
    let mut request = v2_agent_request(task, spec, &execution);
    if let Some(repository_root) = repository_root_override {
        request.repository_root = Some(repository_root);
    }
    let call_client = client.with_provider_tier(provider_tier_for_v2_request(&request));
    adapter
        .run_with_repair(&call_client, &request)
        .await
        .map_err(|err| WorkflowError::StageFailed(err.to_string()))
}

pub(super) fn provider_tier_for_v2_request(
    request: &archon_workflow::WorkflowV2AgentRequest,
) -> ProviderTier {
    match request.role.to_ascii_lowercase().as_str() {
        "planner" => ProviderTier::Planner,
        "researcher" => ProviderTier::Researcher,
        "coder" | "implementation" => ProviderTier::Coder,
        "critic" => ProviderTier::Critic,
        "reducer" => ProviderTier::Reducer,
        "cheap" => ProviderTier::Cheap,
        "local" | "tool" => ProviderTier::Local,
        "vision" => ProviderTier::Vision,
        _ => match request.call.method {
            WorkflowV2HostMethod::Implementation => ProviderTier::Coder,
            WorkflowV2HostMethod::Reduce | WorkflowV2HostMethod::FinalReport => {
                ProviderTier::Reducer
            }
            WorkflowV2HostMethod::QualityGate | WorkflowV2HostMethod::HumanGate => {
                ProviderTier::Critic
            }
            WorkflowV2HostMethod::Tool
            | WorkflowV2HostMethod::SaveArtifact
            | WorkflowV2HostMethod::RequireArtifact
            | WorkflowV2HostMethod::Checkpoint => ProviderTier::Local,
            WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel => ProviderTier::Coder,
            WorkflowV2HostMethod::Agent => ProviderTier::Researcher,
        },
    }
}

async fn run_read_only_v2_fanout(
    task: &str,
    spec: &WorkflowSpec,
    execution: WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: &WorkflowV2ResultStore,
    store_for_control: &WorkflowStore,
    run_id: &str,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let items = fanout_items_for_call(&execution, v2_store)?;
    let item_order = branch_item_order(&items);
    let (reused_outcomes, pending_items) =
        split_reusable_branch_outcomes(v2_store, &execution.call.id, items)?;
    let max_parallelism =
        client.read_only_fanout_parallelism(execution.call.options.max_parallelism);
    let scheduler = WorkflowV2Scheduler::new(WorkflowV2SchedulerConfig {
        max_parallelism,
        ..WorkflowV2SchedulerConfig::default()
    });
    let task = task.to_string();
    let spec = spec.clone();
    let parent_call_id = execution.call.id.clone();
    let branch_parent_call_id = execution.call.id.clone();
    let branch_store = v2_store.clone();
    let branch_control_store = store_for_control.clone();
    let branch_run_id = run_id.to_string();
    let run_report = if pending_items.is_empty() {
        WorkflowV2FanoutReport {
            outcomes: Vec::new(),
            max_parallelism,
            peak_parallelism: 0,
            cancelled: false,
        }
    } else {
        scheduler
            .run_read_only_fanout_observed(
                pending_items,
                move |outcome| {
                    branch_store.save_branch_outcome(&branch_parent_call_id, outcome)?;
                    Ok(())
                },
                |branch| {
                    let adapter = adapter.clone();
                    let task = task.clone();
                    let spec = spec.clone();
                    let parent_call_id = parent_call_id.clone();
                    let control_store = branch_control_store.clone();
                    let run_id = branch_run_id.clone();
                    async move {
                        poll_v2_run_control(&control_store, &run_id, &branch.id)?;
                        let branch_execution = WorkflowV2CallExecution {
                            call: branch.call.clone(),
                            input: branch.input.clone(),
                            depends_on: vec![parent_call_id],
                        };
                        let result = run_single_v2_agent_call(
                            &task,
                            &spec,
                            &branch_execution,
                            &adapter,
                            client,
                            None,
                        )
                        .await?;
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
    let branch_artifact_paths = report
        .outcomes
        .iter()
        .map(|outcome| {
            v2_store
                .branch_outcome_path(&execution.call.id, &outcome.item_id)
                .display()
                .to_string()
        })
        .collect::<Vec<_>>();
    let mut result = result_from_fanout_report(&execution.call, report);
    if let Some(object) = result.data.as_object_mut() {
        object.insert(
            "branch_artifact_paths".to_string(),
            serde_json::json!(branch_artifact_paths),
        );
    }
    Ok(result)
}

pub(super) fn split_reusable_branch_outcomes(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
    items: Vec<WorkflowV2FanoutItem>,
) -> archon_workflow::WorkflowResult<(Vec<WorkflowV2BranchOutcome>, Vec<WorkflowV2FanoutItem>)> {
    let mut reused = Vec::new();
    let mut pending = Vec::new();
    for item in items {
        match v2_store.load_branch_outcome(call_id, &item.id)? {
            Some(outcome) if reusable_branch_outcome(&outcome) => reused.push(outcome),
            _ => pending.push(item),
        }
    }
    Ok((reused, pending))
}

pub(super) fn reusable_branch_outcome(outcome: &WorkflowV2BranchOutcome) -> bool {
    matches!(
        outcome.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) && outcome
        .result
        .as_ref()
        .is_some_and(|result| result.status == outcome.status && result.validate().is_ok())
}

pub(super) fn branch_item_order(items: &[WorkflowV2FanoutItem]) -> BTreeMap<String, usize> {
    items
        .iter()
        .enumerate()
        .map(|(idx, item)| (item.id.clone(), idx))
        .collect()
}

pub(super) fn sort_branch_outcomes_by_order(
    outcomes: &mut [WorkflowV2BranchOutcome],
    order: &BTreeMap<String, usize>,
) {
    outcomes.sort_by_key(|outcome| order.get(&outcome.item_id).copied().unwrap_or(usize::MAX));
}

#[cfg(test)]
mod branch_cache_tests {
    use super::*;
    use archon_workflow::{
        WorkflowV2BranchOutcome, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutItem,
        WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result,
        WorkflowV2ResultStore, WorkflowV2Status,
    };

    #[test]
    fn reusable_branch_outcomes_preserve_siblings_for_restart_item() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let mut accepted = WorkflowV2Result::accepted("accepted sibling branch");
        accepted.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "sibling branch has concrete cached evidence",
        ));
        store
            .save_branch_outcome(
                "review",
                &WorkflowV2BranchOutcome {
                    item_id: "review-a".to_string(),
                    role: "critic".to_string(),
                    status: WorkflowV2Status::Accepted,
                    result: Some(accepted),
                    error: None,
                },
            )
            .expect("save branch");
        let base_call = WorkflowV2HostCall {
            id: "review".to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        };
        let items = vec![
            WorkflowV2FanoutItem::read_only(
                "review-a",
                "critic",
                base_call.clone(),
                serde_json::json!({"item": "a"}),
            ),
            WorkflowV2FanoutItem::read_only(
                "review-b",
                "critic",
                base_call,
                serde_json::json!({"item": "b"}),
            ),
        ];

        let (reused, pending) =
            split_reusable_branch_outcomes(&store, "review", items).expect("split branches");

        assert_eq!(reused.len(), 1);
        assert_eq!(reused[0].item_id, "review-a");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "review-b");
    }

    #[test]
    fn branch_outcomes_needing_review_are_not_reused() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let result = WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: "review found unresolved issues".to_string(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Review,
                "review branch requires remediation before reuse",
            )],
            ..WorkflowV2Result::default()
        };
        store
            .save_branch_outcome(
                "review",
                &WorkflowV2BranchOutcome {
                    item_id: "review-a".to_string(),
                    role: "critic".to_string(),
                    status: WorkflowV2Status::NeedsReview,
                    result: Some(result),
                    error: None,
                },
            )
            .expect("save branch");
        let base_call = WorkflowV2HostCall {
            id: "review".to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        };
        let items = vec![WorkflowV2FanoutItem::read_only(
            "review-a",
            "critic",
            base_call,
            serde_json::json!({"item": "a"}),
        )];

        let (reused, pending) =
            split_reusable_branch_outcomes(&store, "review", items).expect("split branches");

        assert!(reused.is_empty());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "review-a");
    }
}
