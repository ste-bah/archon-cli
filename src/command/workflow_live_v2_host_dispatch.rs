async fn execute_v2_live_call(
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
    if execution.call.method == WorkflowV2HostMethod::Tool {
        return execute_declared_local_tool(execution, v2_store, task_universe);
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
            )
            .await
        }
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel => {
            run_write_capable_v2_fanout(
                task,
                runtime.target_repository_root.as_deref(),
                execution,
                adapter,
                client,
                v2_store,
                store_for_control,
                run_id,
                workspace_boundary_supported,
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
            )
            .await
        }
    }
}

fn execute_declared_local_tool(
    execution: WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let tool_name = declared_local_tool_name(&execution).ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "w.tool('{}') is missing required allowlisted local tool name in options.tool",
            execution.call.id
        ))
    })?;
    let method = allowlisted_local_tool_method(&tool_name).ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "w.tool('{}') declared unknown local tool '{}'; allowed generated V2 tools are checkpoint, saveArtifact, and requireArtifact",
            execution.call.id, tool_name
        ))
    })?;
    let delegated = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            method,
            ..execution.call
        },
        input: execution.input,
        depends_on: execution.depends_on,
    };
    execute_local_host_call(&delegated, v2_store, task_universe)?.ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "w.tool('{}') could not execute allowlisted local tool '{}'",
            delegated.call.id, tool_name
        ))
    })
}

fn declared_local_tool_name(execution: &WorkflowV2CallExecution) -> Option<String> {
    execution
        .call
        .options
        .extra
        .get("tool")
        .or_else(|| execution.call.options.extra.get("name"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            execution
                .input
                .get("options")
                .and_then(|options| options.get("tool").or_else(|| options.get("name")))
                .and_then(serde_json::Value::as_str)
        })
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

fn allowlisted_local_tool_method(tool_name: &str) -> Option<WorkflowV2HostMethod> {
    match tool_name.trim().to_ascii_lowercase().as_str() {
        "checkpoint" => Some(WorkflowV2HostMethod::Checkpoint),
        "saveartifact" | "save_artifact" => Some(WorkflowV2HostMethod::SaveArtifact),
        "requireartifact" | "require_artifact" => Some(WorkflowV2HostMethod::RequireArtifact),
        _ => None,
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
    target_repository_root: Option<String>,
    execution: &WorkflowV2CallExecution,
    adapter: &WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: Option<&WorkflowV2ResultStore>,
    task_universe: Option<&WorkflowV2TaskUniverse>,
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
    )
    .await
}

async fn run_single_v2_agent_call_in_repository(
    task: &str,
    target_repository_root: Option<String>,
    execution: &WorkflowV2CallExecution,
    adapter: &WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    v2_store: Option<&WorkflowV2ResultStore>,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    repository_root_override: Option<String>,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let execution = match v2_store {
        Some(store) => execution_with_resolved_source(execution, store)?,
        None => execution.clone(),
    };
    let repository_root = repository_root_override.or(target_repository_root);
    let mut request = v2_agent_request(task, repository_root, &execution, task_universe);
    if let Some(store) = v2_store {
        let mut context = archon_workflow::project_artifact_context_from_v2_root(store.root());
        context.add_artifact_requirements(&request.input);
        request.project_artifacts = context;
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
        Err(err) => Err(WorkflowError::StageFailed(err.to_string())),
    }
}

async fn run_v2_agent_call_with_rejected_output_log(
    adapter: &WorkflowV2AgentAdapter,
    client: &LiveV2AgentClient,
    request: &archon_workflow::WorkflowV2AgentRequest,
    v2_store: Option<&WorkflowV2ResultStore>,
) -> Result<WorkflowV2Result, WorkflowV2AgentError> {
    let first = client
        .run_agent_request(request, adapter.build_prompt_parts(request).invocation)
        .await?;
    match adapter.parse_agent_output(request, &first) {
        Ok(result) => {
            save_rejected_write_result(v2_store, request, "first", &first, &result);
            Ok(result)
        }
        Err(first_error) => {
            save_rejected_write_output(v2_store, request, "first", &first, &first_error);
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
}

include!("workflow_live_v2_host_dispatch_repair.rs");

fn save_rejected_write_output(
    v2_store: Option<&WorkflowV2ResultStore>,
    request: &archon_workflow::WorkflowV2AgentRequest,
    attempt: &str,
    body: &str,
    error: &WorkflowV2AgentError,
) {
    if !request.is_write_capable() {
        return;
    }
    let Some(store) = v2_store else {
        return;
    };
    let record = WorkflowV2RejectedOutput {
        attempt: attempt.to_string(),
        error: error.to_string(),
        raw_body: body.to_string(),
    };
    let _ = store.append_rejected_output(&request.call.id, record);
}

fn save_rejected_write_result(
    v2_store: Option<&WorkflowV2ResultStore>,
    request: &archon_workflow::WorkflowV2AgentRequest,
    attempt: &str,
    body: &str,
    result: &WorkflowV2Result,
) {
    if !result_has_rejected_write_output(result) {
        return;
    }
    save_rejected_write_output(
        v2_store,
        request,
        attempt,
        body,
        &WorkflowV2AgentError::InvalidResult(result.summary.clone()),
    );
}

fn result_has_rejected_write_output(result: &WorkflowV2Result) -> bool {
    result.residual_gaps.iter().any(|gap| {
        gap.id.starts_with("invalid_write_branch_output_")
            || gap.description.contains("patch is empty")
            || gap.description.contains("output not usable")
            || gap.description.contains("verification blocked after patch")
            || gap.description.contains("exceeds max")
    })
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

fn generated_prd_contract_repairable_reduce(
    call: &WorkflowV2HostCall,
    error: &WorkflowV2AgentError,
) -> bool {
    call.method == WorkflowV2HostMethod::Reduce
        && generated_prd_contract_reduce_id(&call.id)
        && repairable_agent_contract_error(error)
}

fn generated_prd_contract_reduce_id(call_id: &str) -> bool {
    call_id == "canonical-implementation-inventory"
        || call_id.starts_with("inventory-shape-repair-")
        || call_id.starts_with("task-universe-reconcile-")
        || call_id.starts_with("dependency-graph-repair-")
        || call_id.starts_with("target-file-discovery-")
        || call_id.starts_with("verification-requirements-discovery-")
        || call_id.starts_with("artifact-requirements-discovery-")
        || call_id.starts_with("provider-environment-discovery-")
        || call_id.starts_with("evidence-repair-")
}

fn repairable_agent_contract_error(error: &WorkflowV2AgentError) -> bool {
    match error {
        WorkflowV2AgentError::MalformedOutput(_)
        | WorkflowV2AgentError::InvalidResult(_)
        | WorkflowV2AgentError::RestoredContextSummary
        | WorkflowV2AgentError::ConfirmationQuestion
        | WorkflowV2AgentError::ReadOnlyChangedFiles => true,
        WorkflowV2AgentError::RepairExhausted {
            first_error,
            repair_error,
        } => {
            repairable_agent_contract_error(first_error)
                && repairable_agent_contract_error(repair_error)
        }
        WorkflowV2AgentError::Transport(_)
        | WorkflowV2AgentError::PlanOnlyImplementation
        | WorkflowV2AgentError::ImplementationAcceptedWithoutChanges
        | WorkflowV2AgentError::ImplementationNoopWithoutTaskCoverage
        | WorkflowV2AgentError::ImplementationNoopMissingProjectArtifactEvidence
        | WorkflowV2AgentError::ImplementationChangedFilesOutsideOwnership(_) => false,
    }
}

fn repairable_generated_reduce_result(
    call_id: &str,
    error: &WorkflowV2AgentError,
) -> WorkflowV2Result {
    let message = format!(
        "generated PRD reducer '{}' returned repairable malformed contract output: {}",
        call_id, error
    );
    WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: message.clone(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            message.clone(),
        )],
        residual_gaps: vec![WorkflowV2ResidualGap {
            id: format!(
                "repairable_generated_reduce_contract_{}",
                sanitize_generated_contract_gap_id(call_id)
            ),
            description: message.clone(),
            severity: Some("review".to_string()),
        }],
        data: serde_json::json!({
            "items": [],
            "unresolved_issues": [{
                "kind": "inventory_shape_repair",
                "field": "result_envelope",
                "message": message,
                "item_id": null,
                "canonical_task_ids": []
            }],
            "repairable_schema_failure": true,
            "failed_call_id": call_id,
        }),
        ..WorkflowV2Result::default()
    }
}

fn sanitize_generated_contract_gap_id(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

#[cfg(test)]
#[path = "workflow_live_v2_host_dispatch_rejected_output_tests.rs"]
mod rejected_output_tests;
