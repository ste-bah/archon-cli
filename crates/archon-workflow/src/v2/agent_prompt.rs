use serde::Serialize;

use super::agent_adapter::{
    FINAL_OUTPUT_RULE, IMPLEMENTATION_RULES, READ_ONLY_RULES, RESULT_SCHEMA,
    WorkflowV2AgentRequest, WorkflowV2PromptParts, write_mode_label,
};

pub(super) fn build_prompt_parts(request: &WorkflowV2AgentRequest) -> WorkflowV2PromptParts {
    let (stable_input, invocation_input) = split_stable_input(&request.input);
    let input = compact_json(&invocation_input);
    let stable_input = compact_json(&stable_input);
    let target_files = compact_json(&request.target_files);
    let target_ownership_scopes = compact_json(&request.target_ownership_scopes);
    let artifact_roots = compact_json(&request.project_artifacts.artifact_roots);
    let constraints = compact_json(&request.constraints);
    let write_rules = if request.is_write_capable() {
        IMPLEMENTATION_RULES
    } else {
        READ_ONLY_RULES
    };
    let final_output_rule = if request.is_write_capable() {
        FINAL_OUTPUT_RULE
    } else {
        ""
    };
    let project_artifact_paths = super::project_artifact_prompt::project_artifact_prompt_section(
        &request.input,
        &request.call.options.required_artifacts,
        request.is_write_capable(),
        &request.project_artifacts,
    );

    let stable_prefix = format!(
        "## Archon Workflow V2 Stable Context\n\
         ## Constraints\n```json\n{constraints}\n```\n\n\
         ## Task Universe\n```json\n{stable_input}\n```\n\n\
         ## Execution Rules\n\
         - Execute the requested work now; do not ask a confirmation question.\n\
         - Return exactly one JSON object and no markdown fence, prose prefix, or prose suffix.\n\
         - Do not return restored-context summaries or previous-session summaries.\n\
         - Do not stop at a plan or proposed next steps for executable work.\n\
         {write_rules}\n\n\
         ## Required JSON Result Envelope\n\
         {RESULT_SCHEMA}\n\n\
         {final_output_rule}",
    );
    let invocation = format!(
        "## Archon Workflow V2 Agent Call\n\
         call_id: {call_id}\n\
         role: {role}\n\
         write_mode: {write_mode}\n\
         repository_root: {repository_root}\n\
         project_artifact_root: {project_artifact_root}\n\
         project_artifact_roots: {artifact_roots}\n\
         workflow_branch_evidence_root: {branch_evidence_root}\n\
         target_files: {target_files}\n\
         target_ownership_scopes: {target_ownership_scopes}\n\n\
         {project_artifact_paths}\
         ## Task\n{task}\n\n\
         ## Input\n```json\n{input}\n```",
        call_id = request.call.id,
        role = request.role,
        write_mode = write_mode_label(request.call.write_mode),
        repository_root = request.repository_root.as_deref().unwrap_or("<none>"),
        project_artifact_root = request
            .project_artifacts
            .project_root
            .as_deref()
            .unwrap_or("<none>"),
        branch_evidence_root = request
            .project_artifacts
            .branch_evidence_root
            .as_deref()
            .unwrap_or("<none>"),
        task = request.task,
    );
    WorkflowV2PromptParts {
        stable_prefix,
        invocation,
    }
}

fn compact_json(value: &impl Serialize) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

fn split_stable_input(input: &serde_json::Value) -> (serde_json::Value, serde_json::Value) {
    let mut invocation = input.clone();
    let mut universes = Vec::new();
    extract_task_universes(&mut invocation, &mut universes);
    let stable = match universes.len() {
        0 => serde_json::Value::Null,
        1 => serde_json::json!({"task_universe": universes.remove(0)}),
        _ => serde_json::json!({"task_universes": universes}),
    };
    (stable, invocation)
}

fn extract_task_universes(value: &mut serde_json::Value, universes: &mut Vec<serde_json::Value>) {
    match value {
        serde_json::Value::Object(object) => {
            for key in ["task_universe", "taskUniverse"] {
                if let Some(universe) = object.remove(key) {
                    push_unique_universe(universes, universe);
                }
            }
            for nested in object.values_mut() {
                extract_task_universes(nested, universes);
            }
        }
        serde_json::Value::Array(values) => {
            values.retain_mut(|nested| {
                if is_task_universe(nested) {
                    push_unique_universe(universes, nested.clone());
                    false
                } else {
                    extract_task_universes(nested, universes);
                    true
                }
            });
        }
        _ => {}
    }
}

fn is_task_universe(value: &serde_json::Value) -> bool {
    value.as_object().is_some_and(|object| {
        object.contains_key("schema_version")
            && object.contains_key("source_roots")
            && object.get("tasks").is_some_and(serde_json::Value::is_array)
    })
}

fn push_unique_universe(universes: &mut Vec<serde_json::Value>, universe: serde_json::Value) {
    if !universes.contains(&universe) {
        universes.push(universe);
    }
}
