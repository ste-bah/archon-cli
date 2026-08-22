use super::*;

pub fn v2_agent_request(
    task: &str,
    target_repository_root: Option<String>,
    execution: &WorkflowV2CallExecution,
    task_universe: Option<&crate::task_universe::WorkflowV2TaskUniverse>,
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
    if let Some(universe) = task_universe
        && let Some(carried) = request_task_universe(execution, universe)
    {
        let carried =
            serde_json::to_value(&carried).expect("WorkflowV2TaskUniverse must serialize to JSON");
        if !contains_task_universe(&input, &carried) {
            match &mut input {
                serde_json::Value::Object(object) => {
                    object.insert("task_universe".to_string(), carried);
                }
                serde_json::Value::Array(values) => values.push(carried),
                _ => input = serde_json::json!([input, carried]),
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

/// The task universe this call's request carries, if any.
///
/// Implementation branches need one, because they are the calls being asked to
/// satisfy a task and a task's acceptance criteria live nowhere else. Before
/// they received it the prompt layer's contract context had nothing to read:
/// it digests the universes it finds in the request, and finding none produced
/// an empty block indistinguishable from a task that declares nothing.
///
/// What they receive is SCOPED to the tasks the branch claims, and that is a
/// correctness requirement rather than a size optimisation. `request.input` is
/// walked recursively by the enforcement paths, not just read at its top level:
/// `project_artifact_contract::artifact_requirement_paths` harvests every
/// `artifact_requirements` key it can reach and
/// `agent_adapter_a::collect_required_tool_names` every `required_tools` key.
/// A whole universe in the input therefore makes one branch answerable for the
/// artifacts and tools of EVERY task in the decomposition — its result is
/// demoted to Failed for an artifact another task declared, and a Noop verdict
/// becomes unreachable because evidence is demanded for tools its own task
/// never named. `agent_prompt_contract` already scopes what the agent is SHOWN
/// to the same claimed ids; leaving the input unscoped meant an agent shown its
/// own contract and judged against all of them.
fn request_task_universe(
    execution: &WorkflowV2CallExecution,
    universe: &crate::task_universe::WorkflowV2TaskUniverse,
) -> Option<crate::task_universe::WorkflowV2TaskUniverse> {
    // Checked before the method, so a reconciliation call keeps the whole set
    // whatever method it is derived as.
    if carries_whole_task_universe(&execution.call.id) {
        return Some(universe.clone());
    }
    if execution.call.method == WorkflowV2HostMethod::Implementation {
        return scoped_task_universe(&execution.input, universe);
    }
    None
}

/// A completion-claim repair reconciles the run's claims against the whole
/// decomposition, so the whole set genuinely is its subject. It is read-only
/// and carries no declared artifacts or tools of its own, so the enforcement
/// paths above have nothing to over-apply.
fn carries_whole_task_universe(call_id: &str) -> bool {
    call_id
        .rsplit_once("-transport-retry-")
        .map_or(call_id, |(base, _)| base)
        .starts_with("completion-claim-repair-")
}

/// The universe cut to the tasks this branch claims.
///
/// A branch that claims nothing resolvable gets `None` — no universe at all —
/// rather than the whole one. Falling back to everything is exactly the
/// behaviour that made a branch answerable for every task's contract, so the
/// fallback would re-create the defect at the one moment the claim is unknown.
/// An agent that receives no contract is a visible gap; an agent judged against
/// fifteen other tasks' contracts fails in a way that reads as its own error.
fn scoped_task_universe(
    input: &serde_json::Value,
    universe: &crate::task_universe::WorkflowV2TaskUniverse,
) -> Option<crate::task_universe::WorkflowV2TaskUniverse> {
    let claimed = crate::v2::branch_stamping::branch_canonical_task_ids(input);
    if claimed.is_empty() {
        return None;
    }
    let tasks = universe
        .tasks
        .iter()
        .filter(|task| task_is_claimed(task, &claimed))
        .cloned()
        .collect::<Vec<_>>();
    if tasks.is_empty() {
        return None;
    }
    Some(crate::task_universe::WorkflowV2TaskUniverse {
        schema_version: universe.schema_version.clone(),
        source_roots: universe.source_roots.clone(),
        tasks,
    })
}

/// Matched on the canonical id or any declared alias — the same test
/// `agent_prompt_contract` applies, so what a branch is shown and what it is
/// judged against cannot drift apart.
fn task_is_claimed(
    task: &crate::task_universe::WorkflowV2TaskUniverseTask,
    claimed: &[String],
) -> bool {
    std::iter::once(&task.canonical_task_id)
        .chain(task.aliases.iter())
        .any(|name| claimed.iter().any(|id| id == name))
}

pub(super) fn contains_task_universe(
    value: &serde_json::Value,
    authoritative: &serde_json::Value,
) -> bool {
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

pub(super) fn target_ownership_scopes(
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

pub(super) fn call_declares_items_output(call: &WorkflowV2HostCall) -> bool {
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
