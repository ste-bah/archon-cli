pub(super) fn v2_agent_request(
    task: &str,
    target_repository_root: Option<String>,
    execution: &WorkflowV2CallExecution,
    task_universe: Option<&super::super::workflow_live_task_universe::WorkflowV2TaskUniverse>,
) -> WorkflowV2AgentRequest {
    let mut constraints = vec![
        "Return exactly one typed WorkflowV2Result JSON object.".to_string(),
        "Do not return markdown, prose-only summaries, or plan-only implementation text."
            .to_string(),
        // Branches run in a headless fanout, so agents must not wait for confirmation.
        "You are a fully autonomous, non-interactive agent. No human will read or reply to your output. Never ask for confirmation or approval, and never wait for or request a \"proceed\", \"go ahead\", or \"do it\" — there is nobody to answer. Complete the assigned work now and report it in the JSON result.".to_string(),
    ];
    if execution.call.write_mode.is_some() {
        // Read-only roles have no write_mode and must not be told to edit files.
        constraints.push(
            "This is a WRITE-CAPABLE branch: you must make the required file edits yourself, now, in your assigned worktree/repository, before returning. Do not stop at a plan and do not describe edits you have not actually made — an accepted result requires the real changes to exist on disk, confirmed by your own commands.".to_string(),
        );
    }
    if call_declares_items_output(&execution.call) {
        constraints.push(
            "This call feeds downstream fanout: put work items in data.items as a flat JSON array of item objects. Do not nest items under dependency_phases, groups, phases, or any other wrapper.".to_string(),
        );
    }
    let mut input = execution.input.clone();
    if needs_request_task_universe(&execution.call.id)
        && let Some(universe) = task_universe
    {
        let universe = serde_json::to_value(universe)
            .expect("WorkflowV2TaskUniverse must serialize to JSON");
        if !contains_task_universe(&input, &universe) {
            match &mut input {
                serde_json::Value::Object(object) => {
                    object.insert("task_universe".to_string(), universe);
                }
                serde_json::Value::Array(values) => values.push(universe),
                _ => input = serde_json::json!([input, universe]),
            }
        }
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
        input,
        repository_root: target_repository_root,
        project_artifacts: Default::default(),
        target_files: execution.call.options.target_files.clone(),
        target_ownership_scopes: target_ownership_scopes(&execution.call.options.extra),
    }
}

fn needs_request_task_universe(call_id: &str) -> bool {
    call_id
        .rsplit_once("-transport-retry-")
        .map_or(call_id, |(base, _)| base)
        .starts_with("completion-claim-repair-")
}

fn contains_task_universe(value: &serde_json::Value, authoritative: &serde_json::Value) -> bool {
    if value == authoritative {
        return true;
    }
    match value {
        serde_json::Value::Object(object) => object
            .values()
            .any(|value| contains_task_universe(value, authoritative)),
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| contains_task_universe(value, authoritative)),
        _ => false,
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
