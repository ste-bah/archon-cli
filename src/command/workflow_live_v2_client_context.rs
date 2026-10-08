use super::*;

impl LiveV2AgentClient {
    pub(in super::super) fn with_owner_store(
        mut self,
        store: archon_workflow::WorkflowStore,
    ) -> Self {
        self.owner_store = Some(store);
        self
    }
    pub(super) async fn admit_provider<T>(
        &self,
        work: impl std::future::Future<Output = archon_workflow::WorkflowResult<T>>,
    ) -> archon_workflow::WorkflowResult<T> {
        match &self.owner_store {
            Some(store) => store.execute_owned(&self.run_id, work).await,
            None => work.await,
        }
    }

    pub(in super::super) fn fanout_parallelism(&self, requested: Option<usize>) -> usize {
        read_only_v2_fanout_parallelism(requested, live_v2_subagent_max_concurrency())
    }

    pub(in super::super) fn read_only_fanout_parallelism(&self, requested: Option<usize>) -> usize {
        self.fanout_parallelism(requested)
    }
}

/// The fixed author's attempt deadline over the call and its transient
/// retries: a no-progress window, never a total. It counts execution time
/// only, from when a session of the call takes its subagent slot (a slot wait
/// does not run it), and renews on the novel activity its sessions report
/// (Issue 288).
pub(super) async fn author_attempt_deadline<T>(
    timeout_secs: Option<u64>,
    attempt: impl std::future::Future<Output = T>,
) -> std::result::Result<T, WorkflowV2AgentError> {
    let Some(seconds) = timeout_secs else {
        return Ok(attempt.await);
    };
    archon_tools::subagent_dispatch_clock::within_named(
        std::time::Duration::from_secs(seconds),
        attempt,
    )
    .await
    .map_err(|(cut, last)| {
        WorkflowV2AgentError::Transport(match cut {
            archon_tools::subagent_dispatch_clock::DispatchCut::Execution(_) => format!(
                "author attempt no-progress deadline exceeded after {seconds}s without novel \
                 activity, including transient retries; {last}"
            ),
            never => format!("author attempt deadline: the call was {never}"),
        })
    })
}

pub(super) fn stage_request_for_v2_agent(
    run_id: &str,
    provider_tier: ProviderTier,
    default_repository_root: Option<String>,
    request: &WorkflowV2AgentRequest,
) -> StageRunRequest {
    StageRunRequest {
        run_id: run_id.to_string(),
        stage_id: request.call.id.clone(),
        stage_kind: stage_kind_for_v2_agent(request),
        agent: Some(request.role.clone()),
        task: request.task.clone(),
        attempt: 1,
        provider_tier,
        depends_on: Vec::new(),
        input: stage_input_for_v2_agent(default_repository_root, request),
    }
}

pub(super) fn stage_input_for_v2_agent(
    default_repository_root: Option<String>,
    request: &WorkflowV2AgentRequest,
) -> serde_json::Value {
    let mut input = match request.input.clone() {
        serde_json::Value::Object(object) => serde_json::Value::Object(object),
        value => serde_json::json!({ "input": value }),
    };
    if let Some(object) = input.as_object_mut() {
        if let Some(root) = request
            .repository_root
            .clone()
            .or(default_repository_root)
            .filter(|root| !root.trim().is_empty())
        {
            object.insert(
                "target_repository_root".to_string(),
                serde_json::Value::String(root),
            );
        }
        insert_project_artifact_context(object, request);
        object.insert(
            "stage_task".to_string(),
            serde_json::Value::String(request.task.clone()),
        );
        object.insert(
            "v2_call".to_string(),
            serde_json::json!({
                "id": request.call.id,
                "method": request.call.method.as_str(),
                "role": request.role,
                "write_mode": request.call.write_mode,
                "target_files": request.target_files,
            }),
        );
        if request.call.write_mode.is_some() {
            object.insert(
                "write_coordination".to_string(),
                serde_json::json!({
                    "enabled": matches!(
                        request.call.write_mode,
                        Some(WorkflowV2WriteMode::Coordinated | WorkflowV2WriteMode::Worktree)
                    ),
                    "mode": request.call.write_mode,
                    "target_files": request.target_files,
                }),
            );
        }
    }
    input
}

pub(super) fn insert_project_artifact_context(
    object: &mut serde_json::Map<String, serde_json::Value>,
    request: &WorkflowV2AgentRequest,
) {
    if request.project_artifacts.is_empty() {
        return;
    }
    if let Some(root) = request.project_artifacts.project_root.clone() {
        object.insert(
            "project_artifact_root".to_string(),
            serde_json::Value::String(root),
        );
    }
    object.insert(
        "project_artifact_roots".to_string(),
        serde_json::json!(request.project_artifacts.artifact_roots),
    );
    if let Some(root) = request.project_artifacts.branch_evidence_root.clone() {
        object.insert(
            "workflow_branch_evidence_root".to_string(),
            serde_json::Value::String(root),
        );
    }
    let resolved = request
        .project_artifacts
        .project_root
        .as_deref()
        .map(|root| stamp_project_artifact_paths(object, root))
        .unwrap_or_default();
    if !resolved.is_empty() {
        object.insert(
            "project_artifact_paths".to_string(),
            serde_json::json!(resolved),
        );
    }
    if request.is_write_capable() {
        mark_required_bash(object);
    }
}

pub(super) fn mark_required_bash(object: &mut serde_json::Map<String, serde_json::Value>) {
    let extra = object
        .entry("stage_extra".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let Some(extra) = extra.as_object_mut() else {
        return;
    };
    let tools = extra
        .entry("required_tools".to_string())
        .or_insert_with(|| serde_json::json!([]));
    let Some(tools) = tools.as_array_mut() else {
        *tools = serde_json::json!(["Bash"]);
        return;
    };
    if !tools.iter().any(|tool| tool.as_str() == Some("Bash")) {
        tools.push(serde_json::json!("Bash"));
    }
}

pub(super) fn stage_kind_for_v2_agent(request: &WorkflowV2AgentRequest) -> StageKind {
    if request.is_write_capable() {
        return StageKind::Implementation;
    }
    match request.call.method {
        WorkflowV2HostMethod::Reduce | WorkflowV2HostMethod::FinalReport => StageKind::Reduce,
        WorkflowV2HostMethod::QualityGate | WorkflowV2HostMethod::HumanGate => {
            StageKind::QualityGate
        }
        WorkflowV2HostMethod::Checkpoint => StageKind::Checkpoint,
        WorkflowV2HostMethod::Tool
        | WorkflowV2HostMethod::HostCommand
        | WorkflowV2HostMethod::SaveArtifact
        | WorkflowV2HostMethod::RequireArtifact => StageKind::Tool,
        WorkflowV2HostMethod::Implementation => StageKind::Implementation,
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel => StageKind::Fanout,
        WorkflowV2HostMethod::Agent => StageKind::Agent,
    }
}

pub(super) fn v2_system_context() -> &'static str {
    "You are an Archon dynamic workflow stage agent. Return exactly one JSON object matching the Workflow V2 result envelope from the user message."
}

/// One definition, shared with the shape tuner.
///
/// The learner's baseline has to be the cap this function returns, or a
/// "narrowing" could be reported against a number the runtime never used. Two
/// copies of this resolution is how that happens, so there is one.
pub(super) fn live_v2_subagent_max_concurrency() -> Option<usize> {
    crate::command::sona_workflow_shape_tuning::resolved_subagent_cap()
}

pub(super) fn read_only_v2_fanout_parallelism(
    requested: Option<usize>,
    subagent_cap: Option<usize>,
) -> usize {
    let cap = subagent_cap
        .unwrap_or(archon_core::subagent::SubagentManager::DEFAULT_MAX_CONCURRENT)
        .max(1);
    requested.map_or(cap, |requested| requested.max(1).min(cap))
}
