#[path = "workflow_raw_evidence.rs"]
mod raw_evidence;
use super::workflow_live_v2_host_dispatch_contract::*;
use super::*;
#[path = "workflow_live_v2_local_tools.rs"]
mod local_tools;
#[path = "workflow_stage_landing.rs"]
mod stage_landing;
pub(super) use local_tools::execute_declared_local_tool;
#[path = "workflow_live_v2_host_call_records.rs"]
mod call_records;
pub(super) use call_records::{host_call_timeout_record, is_host_call_timeout};
pub(crate) use call_records::{save_rejected_output, save_rejected_write_result};

pub(super) async fn execute_v2_live_call(
    task: &str,
    runtime: &WorkflowV2ScriptRuntime,
    execution: WorkflowV2CallExecution,
    adapter: WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: &WorkflowV2ResultStore,
    store_for_control: &WorkflowStore,
    run_id: &str,
    workspace_boundary_supported: bool,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    source_task_graph: Option<&archon_workflow::WorkflowV2SourceTaskGraph>,
    raw_outcomes_allowed: bool,
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
        if let Some(result) = execute_local_host_call(&local_execution, v2_store, task_universe)? {
            return Ok(result);
        }
    }
    // Obs-32: the authored run's acceptance stage is a host-executed tool
    // call — real checks against the repository, no agent — routed before
    // the allowlisted local pseudo-tools it would otherwise be refused by.
    if super::workflow_live_v3_acceptance::is_acceptance_stage_call(&execution) {
        return super::workflow_live_v3_acceptance::run_acceptance_stage(
            runtime,
            &execution,
            store_for_control,
            run_id,
            task_universe,
        )
        .await;
    }
    if execution.call.method == WorkflowV2HostMethod::Tool {
        return execute_declared_local_tool(execution, v2_store, task_universe);
    }
    if client.audit.is_some()
        && execution.call.write_mode.is_some()
        && !matches!(
            execution.call.method,
            WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel
        )
        && runtime.target_repository_root.is_some()
    {
        return audited_direct::run(
            task,
            runtime,
            execution,
            adapter,
            client,
            v2_store,
            store_for_control,
            run_id,
            workspace_boundary_supported,
            task_universe,
            source_task_graph,
        )
        .await;
    }
    match execution.call.method {
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel
            if execution.call.write_mode.is_none() =>
        {
            run_read_only_v2_fanout(
                task,
                runtime,
                execution,
                adapter,
                client,
                v2_store,
                store_for_control,
                run_id,
                task_universe,
            )
            .await
        }
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel => {
            // Built here, not inside the write layer: the item builder is
            // shared with read-only fan-out and resolves stored source
            // expressions, which is host policy about where items come from.
            let branches = fanout_items_for_call(&execution, v2_store)?;
            run_write_capable_v2_fanout(
                task,
                runtime.target_repository_root.as_deref(),
                execution,
                adapter,
                &super::live_agent_dispatch::LiveAgentDispatch::new(client.clone())
                    .with_generated_config(&runtime.generated_config),
                v2_store,
                store_for_control,
                run_id,
                workspace_boundary_supported,
                branches,
                task_universe,
                source_task_graph,
            )
            .await
        }
        _ => {
            run_single_v2_agent_call(
                task,
                runtime.target_repository_root.clone(),
                &execution,
                &adapter,
                client,
                Some(v2_store),
                task_universe,
                raw_outcomes_allowed,
            )
            .await
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

pub(super) async fn run_single_v2_agent_call(
    task: &str,
    target_repository_root: Option<String>,
    execution: &WorkflowV2CallExecution,
    adapter: &WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: Option<&WorkflowV2ResultStore>,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    raw_outcomes_allowed: bool,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    run_single_v2_agent_call_in_repository(
        task,
        target_repository_root,
        execution,
        adapter,
        client,
        v2_store,
        task_universe,
        None,
        raw_outcomes_allowed,
    )
    .await
}

pub(super) async fn run_single_v2_agent_call_in_repository(
    task: &str,
    target_repository_root: Option<String>,
    execution: &WorkflowV2CallExecution,
    adapter: &WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: Option<&WorkflowV2ResultStore>,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    repository_root_override: Option<String>,
    raw_outcomes_allowed: bool,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let scope = v2_store
        .map(|store| {
            archon_observability::transport::EvidenceScope::new(
                store.root().join("transport.jsonl"),
                &execution.call.id,
            )
        })
        .transpose()
        .map_err(|e| WorkflowError::StageFailed(format!("transport evidence unavailable: {e}")))?;
    let invoke = async {
        let execution = match v2_store {
            Some(store) => execution_with_resolved_source(execution, store)?,
            None => execution.clone(),
        };
        let repository_root = repository_root_override.or(target_repository_root);
        let mut request = v2_agent_request(task, repository_root, &execution, task_universe);
        if let Some(store) = v2_store {
            let mut context = archon_workflow::project_artifact_context_from_v2_root(store.root());
            // A deliverable contract may name repository SOURCE, which does not
            // live under the project artifact root. Without this the completion
            // check reports an existing file as absent — observed live as a task
            // failing on "does not exist" for a 455-line file present in the
            // repository. Existence only; write confinement is unchanged.
            context.repository_root = request.repository_root.clone();
            context.add_artifact_requirements(&request.input);
            // An artifact-only item declares no repository targets, so without this
            // it owns nothing and its agent is refused the deliverable it was
            // dispatched to produce. Admit the exact paths the host parsed from its
            // tasks — one file each, never a directory, never agent-authored.
            if let Some(universe) = task_universe
                && let Some(item) = request.input.get("item")
            {
                context.add_contract_artifact_paths(universe, item);
            }
            // Record what was actually computed. When a branch is rejected for
            // writing "outside declared target_files", this is the difference
            // between reading the inputs and guessing at them after the worktree
            // is gone.
            super::workflow_live_v2_artifact_context_log::record(
                store,
                &request.call.id,
                request.repository_root.as_deref(),
                &context,
            );
            request.project_artifacts = context;
        }
        if request.call.options.result_mode == Some(archon_workflow::AgentResultMode::RawOutcome) {
            if !raw_outcomes_allowed {
                return Err(WorkflowError::PolicyDenied(
                "resultMode rawOutcome is available only to the trusted fixed decomposition run"
                    .to_string(),
            ));
            }
            let mut evidence = v2_store
                .map(|store| {
                    raw_evidence::RawEvidence::start(
                        store.root(),
                        &execution.call.id,
                        &request.task,
                    )
                })
                .transpose()?;
            let records = stage_landing::prepare(&mut request, v2_store)?;
            let response = stage_landing::scope(
                records.clone(),
                client.run_agent_raw_request(&request, request.task.clone()),
            )
            .await;
            if let Some(evidence) = &mut evidence {
                evidence.finish(&response)?;
            }
            let mut outcome =
                response.map_err(|error| WorkflowError::StageFailed(error.to_string()))?;
            if let Some(records) = records {
                if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&outcome.content) {
                    if value.get("records_landed").is_some() {
                        value["tasks"] = records.assemble(&value)?["tasks"].clone();
                        value.as_object_mut().unwrap().remove("records_landed");
                        outcome.content = serde_json::to_string(&value)?;
                    }
                }
            }
            let stop_reason = outcome.stop_reason.ok_or_else(|| {
                WorkflowError::StageFailed(
                    "raw provider outcome returned no typed stop reason".to_string(),
                )
            })?;
            if outcome.content.is_empty() {
                return Err(WorkflowError::StageFailed(
                    "raw provider outcome returned empty content".to_string(),
                ));
            }
            let mut result = WorkflowV2Result::accepted("trusted raw provider outcome captured");
            result.evidence.push(WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Inspection,
                "fixed decomposition author returned provider content and typed stop reason",
            ));
            result.data = serde_json::json!({
                "content": outcome.content,
                "stopReason": stop_reason,
                "tokensIn": outcome.tokens_in,
                "tokensOut": outcome.tokens_out,
            });
            return Ok(result);
        }
        let provider_env = workflow_live_provider_env::prepare_provider_env_for_v2_request(
            &mut request,
            client.provider_env_resolution(),
        )
        .await;
        let call_client = client.with_provider_tier(provider_tier_for_v2_request(&request));
        match run_v2_agent_call_with_rejected_output_log(adapter, &call_client, &request, v2_store)
            .await
        {
            Ok(mut result) => {
                workflow_live_provider_env::stamp_provider_env_result(
                    &mut result,
                    provider_env.as_ref(),
                );
                Ok(result)
            }
            Err(err) if generated_prd_contract_repairable_reduce(&execution.call, &err) => {
                Ok(repairable_generated_reduce_result(&execution.call.id, &err))
            }
            Err(err) if err.is_notification_delivery() => {
                Err(WorkflowError::NotificationDelivery(err.to_string()))
            }
            // The host's own timer ended the session: typed, so no re-ask loop
            // above reads the pipeline's "agent transport failed" wrapper as a
            // provider drop and restarts the session under the same budget.
            Err(err) if is_host_call_timeout(&err.to_string()) => {
                Err(WorkflowError::HostCallTimeout(err.to_string()))
            }
            Err(err) => Err(WorkflowError::StageFailed(err.to_string())),
        }
    };
    match scope {
        Some(scope) => {
            let started = std::time::Instant::now();
            let outcome = scope.run(invoke).await;
            // A cutoff by the host's own timer must be tellable from a provider
            // drop after the fact; both otherwise land as one `agent_call_failed`.
            if let Err(error) = &outcome
                && let Some(row) = host_call_timeout_record(
                    &execution.call.id,
                    &error.to_string(),
                    client.timeout_secs(),
                    client.timeout_source(),
                    started.elapsed().as_secs(),
                )
            {
                scope.record(row);
            }
            scope.record(serde_json::json!({"kind":if outcome.is_err() {"agent_call_failed"} else {"agent_call_completed"},
                "transport_evidence":"transport.jsonl"}));
            scope.check().map_err(|e| {
                WorkflowError::StageFailed(format!("transport evidence write failed: {e}"))
            })?;
            outcome
        }
        None => invoke.await,
    }
}

pub(super) async fn run_v2_agent_call_with_rejected_output_log(
    adapter: &WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    request: &archon_workflow::WorkflowV2AgentRequest,
    v2_store: Option<&WorkflowV2ResultStore>,
) -> Result<WorkflowV2Result, WorkflowV2AgentError> {
    let mut request = request.clone();
    let records = stage_landing::prepare(&mut request, v2_store)
        .map_err(|e| WorkflowV2AgentError::InvalidResult(e.to_string()))?;
    stage_landing::scope(
        records,
        run_logged_agent_inner(adapter, client, &request, v2_store),
    )
    .await
}

async fn run_logged_agent_inner(
    adapter: &WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    request: &archon_workflow::WorkflowV2AgentRequest,
    v2_store: Option<&WorkflowV2ResultStore>,
) -> Result<WorkflowV2Result, WorkflowV2AgentError> {
    let previous = archon_workflow::v2::repair_session::author_previous(request);
    let generation = previous
        .as_ref()
        .map(|(id, _)| id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let prompt = adapter.build_prompt_parts(request).invocation;
    let continuing = previous.is_some();
    let request = previous
        .as_ref()
        .map(|(_, original)| original)
        .unwrap_or(request);
    archon_workflow::v2::repair_session::scope_id(generation, async {
        let first = if continuing {
            client.continue_agent_request(request, prompt).await?
        } else {
            client.run_agent_request(request, prompt).await?
        };
        archon_workflow::v2::repair_session::remember_author(request);
        match adapter.parse_agent_output(request, &first) {
            Ok(result) => {
                save_rejected_write_result(v2_store, request, "first", &first, &result);
                Ok(result)
            }
            Err(first_error) => {
                save_rejected_output(v2_store, request, "first", &first, &first_error);
                run_v2_agent_repair_with_rejected_output_log(
                    adapter,
                    client,
                    request,
                    v2_store,
                    first,
                    first_error,
                )
                .await
            }
        }
    })
    .await
}

pub(crate) fn provider_tier_for_v2_request(
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
            | WorkflowV2HostMethod::HostCommand
            | WorkflowV2HostMethod::SaveArtifact
            | WorkflowV2HostMethod::RequireArtifact
            | WorkflowV2HostMethod::Checkpoint => ProviderTier::Local,
            WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel => ProviderTier::Coder,
            WorkflowV2HostMethod::Agent => ProviderTier::Researcher,
        },
    }
}

#[path = "workflow_repository_audit_direct.rs"]
mod audited_direct;
