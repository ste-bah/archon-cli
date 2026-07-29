use std::collections::BTreeSet;
use std::sync::Arc;

use archon_pipeline::runner::{AgentExecutionRequest, LlmClient, LlmResponse};
use archon_workflow::{StageKind, StageRunRequest, WorkflowError};
use serde_json::{Value, json};

use super::workflow_live_retry;
use super::workflow_live_runner::workflow_stage_system_context;

pub(super) async fn repair_item_output<Fut>(
    llm: &Arc<dyn LlmClient>,
    request: &StageRunRequest,
    agent_request: &AgentExecutionRequest,
    first_response: LlmResponse,
    on_retry: impl FnMut(usize) -> Fut,
) -> archon_workflow::WorkflowResult<LlmResponse>
where
    Fut: std::future::Future<Output = archon_workflow::WorkflowResult<()>>,
{
    let mut repair_request = agent_request.clone();
    repair_request.messages = vec![serde_json::json!({
        "role": "user",
        "content": item_output_repair_prompt(request, &first_response.content),
    })];
    repair_request.system = vec![serde_json::json!({
        "type": "text",
        "text": workflow_stage_system_context(request),
    })];
    let mut repaired =
        workflow_live_retry::run_agent_with_transient_retry(llm, repair_request, on_retry).await?;
    if item_output_needs_schema_repair(request, &repaired.content) {
        if let Some(fallback) =
            fallback_read_only_discovery_items(request, first_response, repaired)
        {
            return Ok(fallback);
        }
        return Err(WorkflowError::StageFailed(format!(
            "stage '{}' declares outputs: [items] but emitted no parseable items or completed_items structure after schema repair retry",
            request.stage_id
        )));
    }

    let mut tool_uses = first_response.tool_uses;
    tool_uses.extend(repaired.tool_uses);
    repaired.tool_uses = tool_uses;
    repaired.tokens_in = repaired.tokens_in.saturating_add(first_response.tokens_in);
    repaired.tokens_out = repaired
        .tokens_out
        .saturating_add(first_response.tokens_out);
    Ok(repaired)
}

pub(super) fn item_output_needs_schema_repair(request: &StageRunRequest, body: &str) -> bool {
    stage_declares_items_output(request) && !has_parseable_items_or_completed_items(body)
}

fn stage_declares_items_output(request: &StageRunRequest) -> bool {
    request
        .input
        .get("stage_extra")
        .is_some_and(value_declares_items_output)
        || value_declares_items_output(&request.input)
}

fn value_declares_items_output(value: &serde_json::Value) -> bool {
    list_contains_ci(value.get("outputs"), "items")
        || value
            .get("produces")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|produces| produces.eq_ignore_ascii_case("items"))
}

fn list_contains_ci(value: Option<&serde_json::Value>, needle: &str) -> bool {
    value
        .and_then(serde_json::Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|value| value.eq_ignore_ascii_case(needle))
        })
}

fn has_parseable_items_or_completed_items(body: &str) -> bool {
    candidate_documents(body).into_iter().any(|doc| {
        serde_json::from_str::<serde_json::Value>(doc)
            .or_else(|_| serde_yaml_ng::from_str::<serde_json::Value>(doc))
            .is_ok_and(|value| {
                value
                    .get("items")
                    .and_then(serde_json::Value::as_array)
                    .is_some()
                    || value
                        .get("completed_items")
                        .and_then(serde_json::Value::as_array)
                        .is_some()
            })
    })
}

fn fallback_read_only_discovery_items(
    request: &StageRunRequest,
    first_response: LlmResponse,
    mut repaired: LlmResponse,
) -> Option<LlmResponse> {
    if request.stage_kind != StageKind::Agent || !request.depends_on.is_empty() {
        return None;
    }
    let mut items = discovery_path_items(request);
    if items.is_empty() {
        items.push(repository_audit_item(request));
    }
    let mut tool_uses = first_response.tool_uses;
    tool_uses.extend(repaired.tool_uses);
    repaired.tool_uses = tool_uses;
    repaired.tokens_in = repaired.tokens_in.saturating_add(first_response.tokens_in);
    repaired.tokens_out = repaired
        .tokens_out
        .saturating_add(first_response.tokens_out);
    repaired.content = json!({
        "items": items,
        "runtime_recovery": {
            "kind": "read_only_discovery_items",
            "stage": request.stage_id,
            "reason": "initial read-only item producer did not return parseable items after schema repair",
            "strictness": "implementation and dependent inventory stages still require agent-provided structured evidence"
        }
    })
    .to_string();
    Some(repaired)
}

fn discovery_path_items(request: &StageRunRequest) -> Vec<Value> {
    let mut paths = BTreeSet::new();
    collect_candidate_paths(&request.task, &mut paths);
    collect_value_paths(&request.input, &mut paths);
    paths
        .into_iter()
        .take(40)
        .enumerate()
        .map(|(idx, path)| {
            let evidence_path = path.clone();
            json!({
                "id": format!("discovery-source-{idx:02}"),
                "kind": "source_path",
                "path": path,
                "purpose": "read-only workflow discovery/audit input recovered from the current workflow task",
                "evidence": [{
                    "path": evidence_path,
                    "summary": "Path was explicitly present in the workflow task or stage input."
                }],
                "dependency_order_notes": "Read-only discovery fallback; no implementation completion is claimed."
            })
        })
        .collect()
}

fn repository_audit_item(request: &StageRunRequest) -> Value {
    let root = request
        .input
        .get("target_repository_root")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(".");
    json!({
        "id": "discovery-repository-audit",
        "kind": "repository_audit",
        "path": root,
        "purpose": "read-only repository audit recovered because the item-producing discovery output was unstructured",
        "evidence": [{
            "path": root,
            "summary": "Repository root was available in the workflow stage input."
        }],
        "dependency_order_notes": "Read-only discovery fallback; no implementation completion is claimed."
    })
}

fn collect_value_paths(value: &Value, paths: &mut BTreeSet<String>) {
    match value {
        Value::String(text) => collect_candidate_paths(text, paths),
        Value::Array(values) => {
            for value in values {
                collect_value_paths(value, paths);
            }
        }
        Value::Object(fields) => {
            for value in fields.values() {
                collect_value_paths(value, paths);
            }
        }
        _ => {}
    }
}

fn collect_candidate_paths(text: &str, paths: &mut BTreeSet<String>) {
    for token in text.split_whitespace() {
        let candidate = token
            .trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '`' | ',' | ';' | ':' | ')' | '(' | '[' | ']' | '{' | '}'
                )
            })
            .trim_end_matches('.');
        if is_path_like(candidate) {
            paths.insert(candidate.to_string());
        }
    }
}

fn is_path_like(candidate: &str) -> bool {
    if candidate.starts_with("http://") || candidate.starts_with("https://") {
        return false;
    }
    (candidate.starts_with('/') || candidate.contains('/'))
        && !candidate.contains("${")
        && candidate.chars().any(|ch| ch.is_ascii_alphanumeric())
}

fn candidate_documents(body: &str) -> Vec<&str> {
    let mut docs = vec![body.trim()];
    let mut rest = body;
    while let Some(start) = rest.find("```") {
        rest = &rest[start + 3..];
        if let Some(newline) = rest.find('\n') {
            rest = &rest[newline + 1..];
        }
        let Some(end) = rest.find("```") else {
            break;
        };
        docs.push(rest[..end].trim());
        rest = &rest[end + 3..];
    }
    docs
}

fn item_output_repair_prompt(request: &StageRunRequest, invalid_output: &str) -> String {
    format!(
        "The previous response for workflow stage '{}' was invalid for this stage contract. This stage declares `outputs: [items]`, but the response did not contain a parseable JSON/YAML object with top-level `items` or `completed_items`.\n\nReturn ONLY one JSON object or YAML document now, with no markdown fences and no prose before or after it.\n\nRequired work shape:\n{{\"items\":[{{\"id\":\"stable-id\",\"task\":\"concise concrete task\",\"evidence\":\"specific evidence from the current stage investigation\",\"target_files\":[\"repository/relative/path.rs\"]}}]}}\n\nRequired no-work proof shape:\n{{\"items\":[],\"completed_items\":[{{\"task_ids\":[\"T001\"],\"status\":\"already_implemented\",\"verified\":true,\"evidence\":[{{\"path\":\"path/to/evidence\",\"summary\":\"why no edit is needed\"}}]}}]}}\n\nRules:\n- Use the task/source evidence already inspected in this workflow stage.\n- Do not ask what to do next.\n- Do not return restored-context summaries.\n- Do not return only `idempotent_noop` or status prose.\n- Do not run broad recursive reads or greps just to repair formatting; only use another tool if it is narrowly necessary to fill a concrete missing field.\n- If and only if the evidence proves there is no downstream implementation work, return `items: []` with matching `completed_items` proof.\n\nPrevious invalid response excerpt:\n{}\n\nStage task:\n{}",
        request.stage_id,
        truncate_for_repair_prompt(invalid_output, 2_000),
        request.task
    )
}

fn truncate_for_repair_prompt(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max_chars) {
        out.push(ch);
    }
    if value.chars().count() > max_chars {
        out.push_str("\n...[truncated]");
    }
    out
}
