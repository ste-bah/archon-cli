use serde_json::Value;

use crate::error::{WorkflowError, WorkflowResult};
use crate::spec::{StageKind, StageSpec, WorkflowSpec};

pub(crate) fn enabled(stage: &StageSpec) -> bool {
    bool_extra(stage, "allow_empty_when_completed")
        || bool_extra(stage, "allow_empty_items_when_completed")
}

pub(crate) fn has_empty_completion_contract(spec: &WorkflowSpec, stage: &StageSpec) -> bool {
    enabled(stage) || downstream_completion_proxy(spec, stage).is_some()
}

pub(crate) fn summary_for_stage(
    spec: &WorkflowSpec,
    stage: &StageSpec,
    bodies: &[String],
) -> WorkflowResult<Option<String>> {
    if enabled(stage) {
        return summary(stage, bodies).map(Some);
    }
    downstream_completion_proxy(spec, stage)
        .map(|proxy| summary(proxy, bodies))
        .transpose()
}

pub(crate) fn summary(stage: &StageSpec, bodies: &[String]) -> WorkflowResult<String> {
    let required = completion_task_ids(stage);
    if required.is_empty() {
        return Err(WorkflowError::StageFailed(format!(
            "implementation fan-out stage '{}' set allow_empty_when_completed but did not set non-empty completion_task_ids",
            stage.id
        )));
    }
    for body in bodies {
        for proof in parse_completed_items(body) {
            if !crate::item_filter::stage_filter_matches(stage, &proof)? {
                continue;
            }
            if covers_required_tasks(&proof, &required)
                && completion_status(&proof)
                && has_concrete_evidence(&proof)
            {
                let scope = proof
                    .get("wave")
                    .or_else(|| proof.get("scope"))
                    .and_then(Value::as_str)
                    .unwrap_or("matching scope");
                return Ok(format!(
                    "upstream inventory proved no missing work for {scope}"
                ));
            }
        }
    }
    Err(WorkflowError::StageFailed(format!(
        "implementation fan-out stage '{}' resolved zero items but no matching completed_items proof covered {:?} with concrete evidence",
        stage.id, required
    )))
}

pub(crate) fn invalid_completed_items_reason(stage_id: &str, body: &str) -> Option<String> {
    for doc in candidate_documents(body) {
        let Some(value) = parse_document(doc) else {
            continue;
        };
        let Some(completed_items) = value.get("completed_items") else {
            continue;
        };
        let Some(items) = completed_items.as_array() else {
            return Some(format!(
                "stage '{stage_id}' emitted invalid completed_items claim: completed_items must be an array"
            ));
        };
        for proof in items {
            if let Some(reason) = completed_claim_error(proof) {
                return Some(format!(
                    "stage '{stage_id}' emitted invalid completed_items claim: {reason}"
                ));
            }
        }
    }
    None
}

fn bool_extra(stage: &StageSpec, key: &str) -> bool {
    stage
        .extra
        .get(key)
        .or_else(|| stage.input.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn completion_task_ids(stage: &StageSpec) -> Vec<String> {
    stage
        .extra
        .get("completion_task_ids")
        .or_else(|| stage.input.get("completion_task_ids"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn parse_completed_items(body: &str) -> Vec<Value> {
    candidate_documents(body)
        .into_iter()
        .filter_map(parse_completed_items_doc)
        .flatten()
        .collect()
}

fn parse_completed_items_doc(body: &str) -> Option<Vec<Value>> {
    parse_document(body).and_then(|value| {
        value
            .get("completed_items")
            .and_then(Value::as_array)
            .cloned()
    })
}

fn parse_document(body: &str) -> Option<Value> {
    serde_json::from_str::<Value>(body)
        .ok()
        .or_else(|| serde_yaml_ng::from_str::<Value>(body).ok())
}

fn covers_required_tasks(proof: &Value, required: &[String]) -> bool {
    let actual = completed_claim_units(proof);
    if actual.is_empty() {
        return false;
    }
    let actual = actual
        .iter()
        .map(String::as_str)
        .flat_map(task_id_aliases)
        .collect::<Vec<_>>();
    required.iter().all(|wanted| {
        let wanted = task_id_aliases(wanted);
        wanted.iter().any(|wanted| actual.contains(wanted))
    })
}

fn completed_claim_error(proof: &Value) -> Option<String> {
    if completed_claim_units(proof).is_empty() {
        return Some("missing task_ids/work_unit_ids".into());
    }
    if proof.get("verified").and_then(Value::as_bool) != Some(true) {
        return Some("missing verified=true".into());
    }
    let Some(status) = proof.get("status").and_then(Value::as_str) else {
        return Some("missing explicit status".into());
    };
    if !completion_status_text(status) {
        return Some(format!("status '{status}' is not a valid completed status"));
    }
    if !has_concrete_evidence(proof) {
        return Some("missing concrete evidence".into());
    }
    None
}

fn completed_claim_units(proof: &Value) -> Vec<String> {
    let mut units = Vec::new();
    for key in [
        "work_unit_ids",
        "task_ids",
        "canonical_task_ids",
        "implemented_work_unit_ids",
        "implemented_task_ids",
        "implemented_canonical_task_ids",
    ] {
        if let Some(values) = proof.get(key).and_then(Value::as_array) {
            units.extend(
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            );
        }
    }
    for key in [
        "work_unit_id",
        "task_id",
        "canonical_task_id",
        "implemented_work_unit_id",
        "implemented_task_id",
        "implemented_canonical_task_id",
    ] {
        if let Some(value) = proof.get(key).and_then(Value::as_str).map(str::trim)
            && !value.is_empty()
        {
            units.push(value.to_string());
        }
    }
    units.sort();
    units.dedup();
    units
}

fn task_id_aliases(raw: &str) -> Vec<String> {
    let canonical = canonical_task_id(raw);
    let mut aliases = vec![raw.trim().to_string()];
    if let Some(id) = canonical {
        aliases.push(id.clone());
        aliases.push(format!("TASK-TDL-{}", id.trim_start_matches('T')));
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

fn canonical_task_id(raw: &str) -> Option<String> {
    let upper = raw.trim().to_ascii_uppercase();
    if upper.len() == 4
        && upper.starts_with('T')
        && upper.chars().skip(1).all(|ch| ch.is_ascii_digit())
    {
        return Some(upper);
    }
    let digits = upper
        .strip_prefix("TASK-TDL-")
        .or_else(|| upper.strip_prefix("TASK_TDL_"))
        .or_else(|| upper.strip_prefix("TDL-"))
        .or_else(|| upper.strip_prefix("TDL_"))
        .map(|suffix| {
            suffix
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>()
        })?;
    (!digits.is_empty()).then(|| format!("T{:0>3}", digits))
}

fn completion_status(proof: &Value) -> bool {
    if proof.get("verified").and_then(Value::as_bool) != Some(true) {
        return false;
    }
    proof
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(completion_status_text)
}

fn completion_status_text(status: &str) -> bool {
    let status = status
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    if [
        "blocked",
        "failed",
        "incomplete",
        "not_already_implemented",
        "not_accepted",
        "not_implemented",
        "denied",
        "rejected",
        "unverified",
    ]
    .iter()
    .any(|needle| status.contains(needle))
    {
        return false;
    }
    matches!(
        status.as_str(),
        "already_implemented"
            | "accepted"
            | "implemented"
            | "complete"
            | "completed"
            | "verified"
            | "satisfied"
            | "no_missing_work"
            | "completed_audit_only"
            | "audit_only"
            | "report_only"
            | "artifact_only"
            | "no_repository_changes_required"
    ) || status.contains("already_implemented")
        || status.contains("audit_only")
        || status.contains("report_only")
        || status.contains("artifact_only")
        || status.starts_with("repository_support_already_implemented")
        || status.contains("deliverables_are_artifact_generation")
        || status.contains("outputs_are_artifacts")
}

fn has_concrete_evidence(proof: &Value) -> bool {
    ["evidence", "concrete_evidence", "verification_evidence"]
        .iter()
        .filter_map(|key| proof.get(*key))
        .any(evidence_value_is_concrete)
}

fn evidence_value_is_concrete(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.iter().any(evidence_value_is_concrete),
        Value::Object(_) => evidence_item_is_concrete(value),
        Value::String(text) => evidence_text_is_concrete(text),
        _ => false,
    }
}

fn evidence_item_is_concrete(item: &Value) -> bool {
    let Some(object) = item.as_object() else {
        return false;
    };
    has_any_text_key(
        object,
        &["path", "artifact_path", "file", "command", "test"],
    ) && has_any_text_key(
        object,
        &["summary", "assertion", "status", "result", "evidence"],
    )
}

fn evidence_text_is_concrete(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    [
        ".rs", ".md", ".toml", ".json", ".yaml", ".yml", ".lock", ".txt", ".csv", ".jsonl",
        ".pine", "src/", "crates/", "tests/", "tasks/", "context/", "cargo ", "git ", "rg ", "::",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn has_any_text_key(object: &serde_json::Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().any(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(has_text)
    })
}

fn has_text(value: &str) -> bool {
    !value.trim().is_empty()
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

fn downstream_completion_proxy<'a>(
    spec: &'a WorkflowSpec,
    stage: &StageSpec,
) -> Option<&'a StageSpec> {
    if stage.kind != StageKind::Fanout || stage.effective_item_kind() == StageKind::Implementation {
        return None;
    }
    let foreach = normalized(stage.foreach.as_deref()?);
    let filter = normalized(stage.filter.as_deref().unwrap_or_default());
    spec.stages.iter().find(|candidate| {
        candidate.id != stage.id
            && candidate.kind == StageKind::Fanout
            && candidate.effective_item_kind() == StageKind::Implementation
            && enabled(candidate)
            && normalized(candidate.foreach.as_deref().unwrap_or_default()) == foreach
            && normalized(candidate.filter.as_deref().unwrap_or_default()) == filter
    })
}

fn normalized(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
