use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::error::{WorkflowError, WorkflowResult};
use crate::fanout::{FanoutItem, extract_items};
use crate::reducers::ReducerInput;
use crate::run::{ArtifactRef, WorkflowRun};
use crate::spec::StageSpec;
use crate::store::WorkflowStore;

const MAX_ARTIFACT_CHARS: usize = 32_000;
const MAX_SOURCE_BYTES: usize = 64 * 1024;
const MAX_SOURCE_FILES: usize = 8;

pub fn stage_input(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Value> {
    Ok(json!({
        "stage_input": stage.input,
        "dependencies": dependency_context(store, run, &stage.depends_on)?,
        "source_files": source_files(store, run, stage)?,
    }))
}

pub fn fanout_input(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
    item: &FanoutItem,
) -> WorkflowResult<Value> {
    Ok(json!({
        "fanout_stage": stage.id,
        "fanout_item_id": item.id,
        "fanout_item": item.payload,
        "context": stage_input(store, run, stage)?,
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
    if let Some(items) = dependency_items(store, run, stage)? {
        return Ok(items);
    }
    let files = source_files(store, run, stage)?;
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
    let rejection = lower.contains("verdict: reject")
        || lower.contains("verdict — reject")
        || lower.contains("verdict - reject")
        || lower.contains("reject — do not sign off")
        || lower.contains("reject - do not sign off")
        || lower.contains("do not sign off")
        || lower.contains("sign-off is not warranted")
        || lower.contains("\"verdict\":\"reject\"")
        || lower.contains("\"status\":\"rejected\"");
    let blocked = lower.contains("\"status\":\"blocked\"")
        || lower.contains("status: blocked")
        || lower.contains("blocked");
    let no_evidence = lower.contains("findings: []")
        || lower.contains("\"findings\":[]")
        || lower.contains("cannot audit")
        || lower.contains("could not audit")
        || lower.contains("can't audit")
        || lower.contains("no source")
        || lower.contains("no file content")
        || lower.contains("insufficient context")
        || lower.contains("missing evidence");
    if rejection {
        return Some("agent output declares reject/do-not-sign-off verdict".to_string());
    }
    blocked
        .then_some(no_evidence)
        .filter(|value| *value)
        .map(|_| "agent output declares blocked or missing evidence".to_string())
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
    Ok(run
        .stages
        .get(stage_id)
        .into_iter()
        .flat_map(|stage| &stage.artifacts)
        .map(|artifact| artifact_value(store, run, artifact))
        .collect::<WorkflowResult<Vec<_>>>()?)
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
    let project_root = project_root(store);
    for body in dependency_bodies(store, run, &dep)? {
        if let Some(items) = parse_items(&body) {
            let items = items
                .into_iter()
                .enumerate()
                .map(|(idx, payload)| FanoutItem {
                    id: format!("{}-{idx}", stage.id),
                    payload: enrich_payload(&project_root, payload),
                })
                .collect::<Vec<_>>();
            if !items.is_empty() {
                return Ok(Some(items));
            }
        }
    }
    Ok(None)
}

fn source_files(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<Vec<Value>> {
    let project_root = project_root(store);
    let mut candidates = BTreeSet::new();
    for candidate in path_candidates(&run.spec.task) {
        candidates.insert(candidate);
    }
    collect_value_paths(&stage.input, &mut candidates);
    Ok(candidates
        .into_iter()
        .filter_map(|candidate| read_source_file(&project_root, &candidate))
        .take(MAX_SOURCE_FILES)
        .collect())
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
        .find_map(|candidate| parse_items_doc(candidate))
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

fn enrich_payload(project_root: &Path, payload: Value) -> Value {
    if let Some(path) = payload.as_str() {
        return read_source_file(project_root, path).unwrap_or_else(|| json!({"value": path}));
    }
    if let Some(path) = payload.get("path").and_then(Value::as_str) {
        if payload.get("content").is_none() {
            if let Some(file) = read_source_file(project_root, path) {
                return merge_payload(payload, file);
            }
        }
    }
    payload
}

fn merge_payload(mut payload: Value, file: Value) -> Value {
    if let (Some(dst), Some(src)) = (payload.as_object_mut(), file.as_object()) {
        for (key, value) in src {
            dst.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    payload
}

fn collect_value_paths(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::String(text) => {
            for candidate in path_candidates(text) {
                out.insert(candidate);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| collect_value_paths(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect_value_paths(item, out)),
        _ => {}
    }
}

fn path_candidates(text: &str) -> Vec<String> {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || "/._-~".contains(ch)))
        .filter(|part| looks_like_path(part))
        .map(|part| part.trim_matches('.').to_string())
        .collect()
}

fn looks_like_path(text: &str) -> bool {
    !text.starts_with("http")
        && (text.contains('/') || text.contains('\\'))
        && [".rs", ".toml", ".md", ".json", ".yaml", ".yml", ".txt"]
            .iter()
            .any(|ext| text.ends_with(ext))
}

fn read_source_file(project_root: &Path, path: &str) -> Option<Value> {
    let raw = Path::new(path);
    let path = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        project_root.join(raw)
    };
    let canonical = path.canonicalize().ok()?;
    let root = project_root.canonicalize().ok()?;
    if !canonical.starts_with(&root) || !canonical.is_file() {
        return None;
    }
    let metadata = canonical.metadata().ok()?;
    let mut file = File::open(&canonical).ok()?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_SOURCE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    let truncated = bytes.len() > MAX_SOURCE_BYTES || metadata.len() > MAX_SOURCE_BYTES as u64;
    bytes.truncate(MAX_SOURCE_BYTES);
    let content = String::from_utf8_lossy(&bytes).to_string();
    Some(json!({
        "path": path.strip_prefix(project_root).unwrap_or(&path).display().to_string(),
        "absolute_path": canonical.display().to_string(),
        "bytes": metadata.len(),
        "truncated": truncated,
        "content": content,
    }))
}

fn project_root(store: &WorkflowStore) -> PathBuf {
    let root = store.root();
    if root.file_name().is_some_and(|name| name == "workflows")
        && root
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == ".archon")
    {
        return root
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf());
    }
    root.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.to_path_buf())
}

fn truncate_chars(body: &str, limit: usize) -> String {
    if body.chars().count() <= limit {
        return body.to_string();
    }
    let mut truncated = body.chars().take(limit).collect::<String>();
    truncated.push_str("\n\n[truncated]");
    truncated
}
