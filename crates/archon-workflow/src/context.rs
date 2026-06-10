use std::fs;

use serde_json::{Value, json};

use crate::error::{WorkflowError, WorkflowResult};
use crate::fanout::{FanoutItem, extract_items};
use crate::reducers::ReducerInput;
use crate::run::{ArtifactRef, WorkflowRun};
use crate::source_context;
use crate::spec::StageSpec;
use crate::store::WorkflowStore;

const MAX_ARTIFACT_CHARS: usize = 32_000;

pub fn stage_input(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Value> {
    let root = source_context::effective_root(store, run);
    Ok(json!({
        "workflow_task": run.spec.task,
        "stage_task": stage.task,
        "stage_extra": stage.extra,
        "stage_input": stage.input,
        "target_repository_root": root.display().to_string(),
        "dependencies": dependency_context(store, run, &stage.depends_on)?,
        "source_files": source_context::stage_source_files(store, run, stage),
    }))
}

pub fn fanout_input(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
    item: &FanoutItem,
) -> WorkflowResult<Value> {
    let context = stage_input(store, run, stage)?;
    let sources = source_context::fanout_source_files(store, run, stage, item, &context);
    Ok(json!({
        "workflow_task": run.spec.task,
        "stage_task": stage.task,
        "stage_extra": stage.extra,
        "stage_input": stage.input,
        "target_repository_root": source_context::effective_root(store, run).display().to_string(),
        "dependencies": context.get("dependencies").cloned().unwrap_or_else(|| json!([])),
        "source_files": sources,
        "fanout_stage": stage.id,
        "fanout_item_id": item.id,
        "fanout_item": item.payload,
        "context": context,
    }))
}

pub fn fanout_items(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Vec<FanoutItem>> {
    if stage.input.get("items").and_then(Value::as_array).is_some() {
        return Ok(extract_items(stage));
    }
    // A fan-out that declares iteration intent via `foreach: ${producer.items}`
    // must resolve to real structured items from its producer. If it does not,
    // fail fast instead of collapsing to a single synthetic item that the agent
    // would (correctly) reject as missing evidence.
    if let Some(dep) = foreach_dependency(stage) {
        let allow_empty = fanout_allows_empty_items(stage);
        return match dependency_items(store, run, stage)? {
            Some(items) if !items.is_empty() || allow_empty => Ok(items),
            Some(_) => Err(WorkflowError::InvalidFanout(format!(
                "fanout stage '{}' declares `foreach` over '{dep}' but that producer emitted an empty `items:` list",
                stage.id
            ))),
            None => Err(WorkflowError::InvalidFanout(format!(
                "fanout stage '{}' declares `foreach` over '{dep}' but that producer emitted no parseable `items:` structure",
                stage.id
            ))),
        };
    }
    let files = source_context::stage_source_files(store, run, stage);
    if let Some(items) = source_file_items(stage, files) {
        return Ok(items);
    }
    Ok(extract_items(stage))
}

pub fn reducer_inputs(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Vec<ReducerInput>> {
    stage
        .depends_on
        .iter()
        .map(|dep| {
            let bodies = dependency_bodies(store, run, dep)?;
            Ok(ReducerInput {
                stage_id: dep.clone(),
                content: bodies.join("\n\n"),
                accepted: run.accepted_stage(dep),
                failed: !run.accepted_stage(dep),
            })
        })
        .collect()
}

pub fn quality_gate_failure(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Option<String>> {
    if stage.depends_on.is_empty() {
        return Ok(None);
    }
    let mut saw_artifact = false;
    for dep in &stage.depends_on {
        for body in dependency_bodies(store, run, dep)? {
            saw_artifact = true;
            if let Some(reason) = output_reports_blocked(&body) {
                return Ok(Some(format!(
                    "dependency `{dep}` reported blocked: {reason}"
                )));
            }
            if let Some(reason) = output_reports_failed_verification(&body) {
                return Ok(Some(format!(
                    "dependency `{dep}` reported failed verification: {reason}"
                )));
            }
        }
    }
    if saw_artifact {
        Ok(None)
    } else {
        Ok(Some(
            "quality gate has no upstream artifacts to inspect".into(),
        ))
    }
}

pub fn output_reports_blocked(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let rejection = reports_reject_verdict(&lower);
    let blocked = reports_explicit_blocked_status(&lower);
    let empty_findings = lower.contains("findings: []") || lower.contains("\"findings\":[]");
    let audit_impossible = lower.contains("cannot audit")
        || lower.contains("could not audit")
        || lower.contains("can't audit")
        || lower.contains("unable to audit")
        || lower.contains("missing required evidence")
        || lower.contains("missing source evidence")
        || lower.contains("source evidence is missing")
        || lower.contains("source files are missing")
        || lower.contains("source files or upstream artifacts are absent")
        || lower.contains("no source evidence")
        || lower.contains("no source files")
        || lower.contains("no file content")
        || lower.contains("insufficient context");
    if rejection {
        return Some("agent output declares reject/do-not-sign-off verdict".to_string());
    }
    if blocked || (empty_findings && audit_impossible) {
        return Some("agent output declares blocked or missing evidence".to_string());
    }
    None
}

pub fn output_reports_failed_verification(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    (json_reports_failed_verification(body) || reports_failed_verification_status(&lower))
        .then(|| "agent output declares failed or unverifiable verification status".to_string())
}

fn reports_reject_verdict(lower: &str) -> bool {
    if lower.contains("\"verdict\":\"reject\"")
        || lower.contains("\"verdict\": \"reject\"")
        || lower.contains("\"status\":\"rejected\"")
        || lower.contains("\"status\": \"rejected\"")
    {
        return true;
    }
    lower.lines().any(|line| {
        let normalized = normalized_status_line(line);
        normalized.starts_with("verdict:reject")
            || normalized.starts_with("verdict-reject")
            || normalized.starts_with("verdict—reject")
            || normalized.starts_with("status:rejected")
            || normalized.starts_with("status=rejected")
    })
}

fn reports_explicit_blocked_status(lower: &str) -> bool {
    if lower.contains("\"status\":\"blocked\"") || lower.contains("\"status\": \"blocked\"") {
        return true;
    }
    let lines: Vec<_> = lower.lines().collect();
    lines.iter().enumerate().any(|(idx, line)| {
        let normalized = normalized_status_line(line);
        normalized.starts_with("status:blocked")
            || normalized.starts_with("-status:blocked")
            || normalized.starts_with("status=blocked")
            || (normalized == "status" && next_line_is_blocked(&lines, idx))
    })
}

fn next_line_is_blocked(lines: &[&str], idx: usize) -> bool {
    lines
        .iter()
        .skip(idx + 1)
        .map(|line| normalized_status_value(line.trim()))
        .find(|line| !line.is_empty())
        .is_some_and(|line| line.starts_with("blocked"))
}

fn reports_failed_verification_status(lower: &str) -> bool {
    let lines: Vec<_> = lower.lines().collect();
    contains_blocking_structured_field(lower)
        || lines.iter().enumerate().any(|(idx, line)| {
            let normalized = normalized_status_line(line);
            starts_with_blocking_status_field(&normalized)
                || (is_verification_status_heading(&normalized)
                    && next_line_has_blocking_status(&lines, idx))
        })
}

fn next_line_has_blocking_status(lines: &[&str], idx: usize) -> bool {
    lines
        .iter()
        .skip(idx + 1)
        .map(|line| normalized_status_value(line.trim()))
        .find(|line| !line.is_empty())
        .is_some_and(|line| starts_with_blocking_status_value(&line))
}

fn json_reports_failed_verification(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .is_some_and(|value| value_reports_failed_verification(&value))
}

fn value_reports_failed_verification(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(field, value)| {
            (is_verification_status_field(field) && value_is_blocking_status(value))
                || value_reports_failed_verification(value)
        }),
        Value::Array(values) => values.iter().any(value_reports_failed_verification),
        _ => false,
    }
}

fn is_verification_status_field(field: &str) -> bool {
    matches!(
        field,
        "status"
            | "verification_status"
            | "overall_status"
            | "overall_result"
            | "result"
            | "final_status"
    )
}

fn value_is_blocking_status(value: &Value) -> bool {
    value
        .as_str()
        .map(normalized_status_value)
        .is_some_and(|value| starts_with_blocking_status_value(&value))
}

fn contains_blocking_structured_field(lower: &str) -> bool {
    const FIELDS: &[&str] = &[
        "status",
        "verification_status",
        "overall_status",
        "overall_result",
        "result",
        "final_status",
    ];
    const VALUES: &[&str] = &[
        "failed",
        "failure",
        "failed_timeout",
        "failed_validation_timeout",
        "completed_with_residual_failure",
        "partial_pass_with_timeout_residual",
        "unverifiable",
        "not_verified",
        "not_fully_verified",
    ];
    FIELDS.iter().any(|field| {
        VALUES.iter().any(|value| {
            lower.contains(&format!("\"{field}\":\"{value}\""))
                || lower.contains(&format!("\"{field}\": \"{value}\""))
        })
    })
}

fn starts_with_blocking_status_field(normalized: &str) -> bool {
    const FIELDS: &[&str] = &[
        "status",
        "verificationstatus",
        "overallstatus",
        "overallresult",
        "result",
        "finalstatus",
    ];
    FIELDS.iter().any(|field| {
        [":", "="].iter().any(|sep| {
            let prefix = format!("{field}{sep}");
            normalized
                .strip_prefix(&prefix)
                .or_else(|| normalized.strip_prefix(&format!("-{prefix}")))
                .is_some_and(starts_with_blocking_status_value)
        })
    })
}

fn is_verification_status_heading(normalized: &str) -> bool {
    matches!(
        normalized,
        "status"
            | "verificationstatus"
            | "overallstatus"
            | "overallresult"
            | "result"
            | "finalstatus"
    )
}

fn starts_with_blocking_status_value(value: &str) -> bool {
    [
        "failed",
        "failure",
        "failedtimeout",
        "failedvalidationtimeout",
        "completedwithresidualfailure",
        "partialpasswithtimeoutresidual",
        "unverifiable",
        "notverified",
        "notfullyverified",
    ]
    .iter()
    .any(|blocking| value.starts_with(blocking))
}

fn normalized_status_value(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| !matches!(ch, '*' | '_' | '`' | ' ' | '-' | '"' | '\'' | ','))
        .collect()
}

fn normalized_status_line(line: &str) -> String {
    line.trim()
        .trim_start_matches('#')
        .chars()
        .filter(|ch| !matches!(ch, '*' | '_' | '`' | ' ' | '"' | '\'' | ','))
        .collect::<String>()
}

fn dependency_context(
    store: &WorkflowStore,
    run: &WorkflowRun,
    deps: &[String],
) -> WorkflowResult<Vec<Value>> {
    deps.iter()
        .map(|dep| {
            let artifacts = dependency_artifacts(store, run, dep)?;
            Ok(json!({
                "stage_id": dep,
                "accepted": run.accepted_stage(dep),
                "artifacts": artifacts,
            }))
        })
        .collect()
}

fn dependency_artifacts(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage_id: &str,
) -> WorkflowResult<Vec<Value>> {
    run.stages
        .get(stage_id)
        .into_iter()
        .flat_map(|stage| &stage.artifacts)
        .map(|artifact| artifact_value(store, run, artifact))
        .collect::<WorkflowResult<Vec<_>>>()
}

fn artifact_value(
    store: &WorkflowStore,
    run: &WorkflowRun,
    artifact: &ArtifactRef,
) -> WorkflowResult<Value> {
    let path = store.run_dir(&run.id).join(&artifact.path);
    let body = fs::read_to_string(&path).map_err(|e| WorkflowError::io(&path, e))?;
    Ok(json!({
        "id": artifact.id,
        "path": artifact.path.display().to_string(),
        "content_hash": artifact.content_hash,
        "accepted": artifact.accepted,
        "content": truncate_chars(&body, MAX_ARTIFACT_CHARS),
    }))
}

fn dependency_bodies(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage_id: &str,
) -> WorkflowResult<Vec<String>> {
    Ok(dependency_artifacts(store, run, stage_id)?
        .into_iter()
        .filter_map(|artifact| {
            artifact
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

fn dependency_items(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Option<Vec<FanoutItem>>> {
    let Some(dep) = foreach_dependency(stage) else {
        return Ok(None);
    };
    for body in dependency_bodies(store, run, &dep)? {
        if let Some(items) = parse_items(&body) {
            let items = items
                .into_iter()
                .enumerate()
                .map(|(idx, payload)| FanoutItem {
                    id: format!("{}-{idx}", stage.id),
                    payload: source_context::enrich_payload(store, run, payload),
                })
                .collect::<Vec<_>>();
            return Ok(Some(items));
        }
    }
    Ok(None)
}

fn fanout_allows_empty_items(stage: &StageSpec) -> bool {
    stage
        .extra
        .get("allow_empty_items")
        .or_else(|| stage.input.get("allow_empty_items"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn source_file_items(stage: &StageSpec, files: Vec<Value>) -> Option<Vec<FanoutItem>> {
    (!files.is_empty()).then(|| {
        files
            .into_iter()
            .enumerate()
            .map(|(idx, payload)| FanoutItem {
                id: format!("{}-{idx}", stage.id),
                payload,
            })
            .collect()
    })
}

fn parse_items(body: &str) -> Option<Vec<Value>> {
    candidate_documents(body)
        .into_iter()
        .find_map(parse_items_doc)
}

fn parse_items_doc(body: &str) -> Option<Vec<Value>> {
    serde_json::from_str::<Value>(body)
        .ok()
        .or_else(|| serde_yaml_ng::from_str::<Value>(body).ok())
        .and_then(|value| value.get("items").and_then(Value::as_array).cloned())
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

fn foreach_dependency(stage: &StageSpec) -> Option<String> {
    let foreach = stage.foreach.as_deref()?.trim();
    let inner = foreach.strip_prefix("${")?.strip_suffix('}')?;
    inner.split('.').next().map(str::to_string)
}

fn truncate_chars(body: &str, limit: usize) -> String {
    if body.chars().count() <= limit {
        return body.to_string();
    }
    let mut truncated = body.chars().take(limit).collect::<String>();
    truncated.push_str("\n\n[truncated]");
    truncated
}
