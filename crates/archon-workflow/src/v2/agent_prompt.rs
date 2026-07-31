use serde::Serialize;

use super::agent_adapter::{
    FINAL_OUTPUT_RULE, IMPLEMENTATION_RULES, READ_ONLY_RULES, RESULT_SCHEMA,
    WorkflowV2AgentRequest, WorkflowV2PromptParts, write_mode_label,
};

pub(super) fn build_prompt_parts(request: &WorkflowV2AgentRequest) -> WorkflowV2PromptParts {
    let (stable_input, invocation_input) = split_stable_input(request);
    WorkflowV2PromptParts {
        stable_prefix: build_stable_prefix(request, &stable_input),
        invocation: build_invocation(request, &invocation_input),
    }
}

fn build_stable_prefix(
    request: &WorkflowV2AgentRequest,
    stable_input: &serde_json::Value,
) -> String {
    let constraints = compact_json(&request.constraints);
    let stable_input = compact_json(stable_input);
    let (write_rules, final_output_rule) = if request.is_write_capable() {
        (IMPLEMENTATION_RULES, FINAL_OUTPUT_RULE)
    } else {
        (READ_ONLY_RULES, "")
    };
    format!(
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
    )
}

fn build_invocation(request: &WorkflowV2AgentRequest, input: &serde_json::Value) -> String {
    let input = compact_json(input);
    let target_files = compact_json(&request.target_files);
    let target_ownership_scopes = compact_json(&request.target_ownership_scopes);
    let artifact_roots = compact_json(&request.project_artifacts.artifact_roots);
    let project_artifact_paths = super::project_artifact_prompt::project_artifact_prompt_section(
        &request.input,
        &request.call.options.required_artifacts,
        request.is_write_capable(),
        &request.project_artifacts,
    );
    format!(
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
    )
}

fn compact_json(value: &impl Serialize) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

fn split_stable_input(request: &WorkflowV2AgentRequest) -> (serde_json::Value, serde_json::Value) {
    let mut invocation = request.input.clone();
    if matches!(
        request.call.method,
        super::WorkflowV2HostMethod::Reduce | super::WorkflowV2HostMethod::FinalReport
    ) {
        digest_wave_evidence(&mut invocation);
    }
    let mut universes = Vec::new();
    extract_task_universes(&mut invocation, &mut universes);
    let base_call_id = base_call_id(&request.call.id);
    let reduced_universe = matches!(
        request.call.method,
        super::WorkflowV2HostMethod::Reduce | super::WorkflowV2HostMethod::FinalReport
    ) && !uses_full_task_universe(base_call_id);
    if uses_task_contract_context(request.call.method, base_call_id) {
        insert_task_contract_context(&mut invocation, &universes);
    }
    if reduced_universe {
        universes = universes
            .into_iter()
            .map(|universe| task_universe_digest(&universe))
            .collect();
    }
    let stable = match universes.len() {
        0 => serde_json::Value::Null,
        1 => serde_json::json!({"task_universe": universes.remove(0)}),
        _ => serde_json::json!({"task_universes": universes}),
    };
    (stable, invocation)
}

fn base_call_id(call_id: &str) -> &str {
    call_id
        .rsplit_once("-transport-retry-")
        .map_or(call_id, |(base, _)| base)
}

fn uses_full_task_universe(call_id: &str) -> bool {
    const FULL_UNIVERSE_PREFIXES: &[&str] = &[
        "inventory-shape-repair-",
        "task-universe-reconcile-",
        "dependency-graph-repair-",
        "target-file-discovery-",
        "verification-requirements-discovery-",
        "artifact-requirements-discovery-",
        "provider-environment-discovery-",
        "evidence-repair-",
    ];
    call_id == "canonical-implementation-inventory"
        || FULL_UNIVERSE_PREFIXES
            .iter()
            .any(|prefix| call_id.starts_with(prefix))
        || call_id == "blocked-malformed-inventory"
        || call_id == "blocked-empty-implementation-inventory"
}

fn uses_task_contract_context(method: super::WorkflowV2HostMethod, call_id: &str) -> bool {
    method == super::WorkflowV2HostMethod::FinalReport
        || call_id.contains("verification")
        || call_id.contains("review")
        || call_id.contains("artifact")
        || call_id.contains("completion-evidence")
        || call_id.starts_with("remediation-")
        || call_id.starts_with("ownership-expansion-")
        || call_id.starts_with("final-evidence-reconciliation-")
        || call_id.starts_with("completion-claim-repair-")
        || call_id == "final-zero-gap-audit"
}

fn insert_task_contract_context(
    invocation: &mut serde_json::Value,
    universes: &[serde_json::Value],
) {
    let tasks = universes
        .iter()
        .flat_map(|universe| {
            universe
                .get("tasks")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
        })
        .map(task_contract_digest)
        .collect::<Vec<_>>();
    let context = serde_json::json!({"tasks":tasks});
    match invocation {
        serde_json::Value::Object(object) => {
            object.insert("task_contract_context".to_string(), context);
        }
        serde_json::Value::Array(values) => {
            values.push(serde_json::json!({"task_contract_context":context}));
        }
        _ => {}
    }
}

fn task_universe_digest(universe: &serde_json::Value) -> serde_json::Value {
    let Some(object) = universe.as_object() else {
        return universe.clone();
    };
    let digests = object
        .get("tasks")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|task| serde_json::Value::Object(task_digest_fields(task)))
        .collect::<Vec<_>>();
    serde_json::json!({
        "schema_version": object.get("schema_version"),
        "source_roots": object.get("source_roots"),
        "tasks": digests,
    })
}

fn task_contract_digest(task: &serde_json::Value) -> serde_json::Value {
    let mut digest = task_digest_fields(task);
    for key in ["acceptance_criteria", "deliverable_contracts"] {
        if let Some(value) = task.get(key) {
            digest.insert(key.to_string(), value.clone());
        }
    }
    serde_json::Value::Object(digest)
}

fn task_digest_fields(task: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    let mut digest = serde_json::Map::new();
    for key in [
        "canonical_task_id",
        "aliases",
        "source_path",
        "dependency_ids",
        "artifact_requirements",
        "required_env_keys",
        "required_tools",
    ] {
        if let Some(value) = task.get(key) {
            digest.insert(key.to_string(), value.clone());
        }
    }
    digest
}

fn digest_wave_evidence(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                if is_evidence_key(key) {
                    if let Some(records) = child.as_array_mut() {
                        digest_old_records(records);
                    }
                } else {
                    digest_wave_evidence(child);
                }
            }
        }
        serde_json::Value::Array(values) => {
            if looks_like_evidence_records(values) {
                digest_old_records(values);
            } else {
                for child in values {
                    digest_wave_evidence(child);
                }
            }
        }
        _ => {}
    }
}

fn looks_like_evidence_records(values: &[serde_json::Value]) -> bool {
    let records = values
        .iter()
        .filter(|value| is_evidence_result_record(value));
    records.count() > 1
        && values
            .iter()
            .all(|value| is_evidence_result_record(value) || is_resultless_evidence_marker(value))
}

fn is_resultless_evidence_marker(value: &serde_json::Value) -> bool {
    value.get("result").is_none()
        && value
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|kind| !kind.trim().is_empty())
}

fn is_evidence_result_record(value: &serde_json::Value) -> bool {
    value.get("result").is_some()
        && value.get("kind").is_some()
        && (value.get("implementationWaveIndex").is_some()
            || value.get("dependencyIteration").is_some()
            || value.get("verificationRepairAttempt").is_some()
            || value.get("verificationRemediationAttempt").is_some()
            || value.get("reviewIteration").is_some()
            || value.get("finalEvidenceIteration").is_some())
}

fn is_evidence_key(key: &str) -> bool {
    matches!(
        key,
        "implementationEvidence"
            | "implementation_evidence"
            | "verificationEvidence"
            | "verification_evidence"
            | "reviewEvidence"
            | "review_evidence"
            | "artifactEvidence"
            | "artifact_evidence"
    )
}

fn digest_old_records(records: &mut [serde_json::Value]) {
    let Some(latest) = records.iter().rposition(is_evidence_result_record) else {
        return;
    };
    for record in &mut records[..latest] {
        if is_evidence_result_record(record) {
            *record = evidence_digest(record);
        }
    }
}

fn evidence_digest(record: &serde_json::Value) -> serde_json::Value {
    let mut digest = serde_json::Map::new();
    for key in [
        "kind",
        "implementationWaveIndex",
        "dependencyIteration",
        "remediationAttempt",
        "verificationRepairAttempt",
        "verificationRemediationAttempt",
        "reviewIteration",
        "finalEvidenceIteration",
    ] {
        if let Some(value) = record.get(key) {
            digest.insert(key.to_string(), value.clone());
        }
    }
    let result = record.get("result").unwrap_or(record);
    digest.insert("result".to_string(), result_digest(result));
    serde_json::Value::Object(digest)
}

fn result_digest(result: &serde_json::Value) -> serde_json::Value {
    let mut digest = serde_json::Map::new();
    for key in ["status", "summary"] {
        if let Some(value) = result.get(key) {
            digest.insert(key.to_string(), value.clone());
        }
    }
    insert_bounded_collection(&mut digest, result, "task_coverage", task_coverage_digest);
    insert_bounded_collection(&mut digest, result, "residual_gaps", residual_gap_digest);
    if let Some(outcomes) = result
        .get("outcomes")
        .or_else(|| result.pointer("/data/outcomes"))
        .and_then(serde_json::Value::as_array)
    {
        insert_bounded_values(&mut digest, "outcomes", outcomes, outcome_digest);
    }
    serde_json::Value::Object(digest)
}

fn insert_bounded_collection(
    digest: &mut serde_json::Map<String, serde_json::Value>,
    source: &serde_json::Value,
    key: &str,
    project: fn(&serde_json::Value) -> serde_json::Value,
) {
    if let Some(values) = source.get(key).and_then(serde_json::Value::as_array) {
        insert_bounded_values(digest, key, values, project);
    }
}

fn insert_bounded_values(
    digest: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    values: &[serde_json::Value],
    project: fn(&serde_json::Value) -> serde_json::Value,
) {
    digest.insert(format!("{key}_count"), serde_json::json!(values.len()));
    let bounded = if values.len() <= 4 {
        values.iter().map(project).collect()
    } else {
        values
            .iter()
            .take(2)
            .chain(values.iter().skip(values.len() - 2))
            .map(project)
            .collect()
    };
    digest.insert(key.to_string(), serde_json::Value::Array(bounded));
}

fn task_coverage_digest(coverage: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "task_id": coverage.get("task_id"),
        "status": coverage.get("status"),
        "summary": coverage.get("summary"),
    })
}

fn residual_gap_digest(gap: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "id": gap.get("id"),
        "severity": gap.get("severity"),
        "description": gap.get("description"),
    })
}

fn outcome_digest(outcome: &serde_json::Value) -> serde_json::Value {
    let result = outcome.get("result").unwrap_or(outcome);
    serde_json::json!({
        "item_id": outcome.get("item_id").or_else(|| outcome.get("id")),
        "canonical_task_ids": outcome
            .get("canonical_task_ids")
            .or_else(|| outcome.get("canonicalTaskIds")),
        "status": result.get("status").or_else(|| outcome.get("status")),
        "summary": result.get("summary").or_else(|| outcome.get("summary")),
    })
}

fn extract_task_universes(value: &mut serde_json::Value, universes: &mut Vec<serde_json::Value>) {
    match value {
        serde_json::Value::Object(object) => {
            for key in ["task_universe", "taskUniverse"] {
                if object.get(key).is_some_and(is_task_universe)
                    && let Some(universe) = object.remove(key)
                {
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
        object
            .get("schema_version")
            .and_then(serde_json::Value::as_str)
            == Some("workflow-v2-task-universe-v1")
            && object
                .get("source_roots")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|roots| roots.iter().all(serde_json::Value::is_string))
            && object
                .get("tasks")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|tasks| tasks.iter().all(is_task_universe_task))
    })
}

fn is_task_universe_task(value: &serde_json::Value) -> bool {
    value
        .get("canonical_task_id")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|id| !id.trim().is_empty())
}

fn push_unique_universe(universes: &mut Vec<serde_json::Value>, universe: serde_json::Value) {
    if !universes.contains(&universe) {
        universes.push(universe);
    }
}
