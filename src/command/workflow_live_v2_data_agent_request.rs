pub(super) fn v2_agent_request(
    task: &str,
    target_repository_root: Option<String>,
    execution: &WorkflowV2CallExecution,
) -> WorkflowV2AgentRequest {
    let mut constraints = vec![
        "Return exactly one typed WorkflowV2Result JSON object.".to_string(),
        "Do not return markdown, prose-only summaries, or plan-only implementation text."
            .to_string(),
    ];
    if call_declares_items_output(&execution.call) {
        constraints.push(
            "This call feeds downstream fanout: put work items in data.items as a flat JSON array of item objects. Do not nest items under dependency_phases, groups, phases, or any other wrapper.".to_string(),
        );
    }
    WorkflowV2AgentRequest {
        call: execution.call.clone(),
        role: execution
            .call
            .options
            .role
            .clone()
            .unwrap_or_else(|| role_for_v2_call(execution.call.method).to_string()),
        task: execution.call.options.task.clone().unwrap_or_else(|| {
            format!(
                "Execute workflow V2 host call '{}' for objective: {}",
                execution.call.id, task
            )
        }),
        constraints,
        input: execution.input.clone(),
        repository_root: target_repository_root,
        project_artifacts: Default::default(),
        target_files: execution.call.options.target_files.clone(),
        target_ownership_scopes: target_ownership_scopes(&execution.call.options.extra),
    }
}

fn target_ownership_scopes(
    extra: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Vec<String> {
    extra
        .get("target_ownership_scopes")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn call_declares_items_output(call: &WorkflowV2HostCall) -> bool {
    call.options
        .extra
        .get("outputs")
        .is_some_and(|outputs| match outputs {
            serde_json::Value::Array(values) => values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|value| value.eq_ignore_ascii_case("items")),
            serde_json::Value::String(value) => value.eq_ignore_ascii_case("items"),
            _ => false,
        })
}

pub(super) fn role_for_v2_call(method: WorkflowV2HostMethod) -> &'static str {
    match method {
        WorkflowV2HostMethod::Implementation => "coder",
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel => "coder",
        WorkflowV2HostMethod::Reduce | WorkflowV2HostMethod::FinalReport => "reducer",
        WorkflowV2HostMethod::QualityGate | WorkflowV2HostMethod::HumanGate => "critic",
        WorkflowV2HostMethod::Tool
        | WorkflowV2HostMethod::SaveArtifact
        | WorkflowV2HostMethod::RequireArtifact
        | WorkflowV2HostMethod::Checkpoint => "tool",
        WorkflowV2HostMethod::Agent => "researcher",
    }
}
