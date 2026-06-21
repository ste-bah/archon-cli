use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::error::{WorkflowError, WorkflowResult};
use crate::fanout::FanoutItem;
use crate::run::WorkflowRun;
use crate::spec::StageSpec;
use crate::store::WorkflowStore;

const MAX_SOURCE_BYTES: usize = 64 * 1024;
const MAX_SOURCE_FILES: usize = 8;

pub(crate) fn stage_source_files(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> Vec<Value> {
    let roots = source_roots(store, run);
    let mut candidates = BTreeSet::new();
    for candidate in path_candidates(&run.spec.task) {
        candidates.insert(candidate);
    }
    if let Some(task) = &stage.task {
        collect_value_paths(&Value::String(task.clone()), &mut candidates);
    }
    collect_value_paths(&stage.input, &mut candidates);
    candidates
        .into_iter()
        .filter_map(|candidate| read_source_file(&roots, &candidate))
        .take(MAX_SOURCE_FILES)
        .collect()
}

pub(crate) fn fanout_source_files(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
    item: &FanoutItem,
    context: &Value,
) -> Vec<Value> {
    let roots = source_roots(store, run);
    let mut sources = context
        .get("source_files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    sources.extend(stage_source_files(store, run, stage));

    let mut candidates = BTreeSet::new();
    let mut targets = BTreeSet::new();
    collect_value_paths(&item.payload, &mut candidates);
    for key in [
        "target_files",
        "expected_target_files",
        "target_file",
        "target_path",
    ] {
        collect_string_values(item.payload.get(key), &mut targets);
    }
    candidates.extend(targets.iter().cloned());

    for candidate in candidates {
        if let Some(file) = read_source_file(&roots, &candidate) {
            sources.push(file);
        } else if targets.contains(&candidate) {
            sources.push(missing_source_file(&roots, &candidate));
        }
    }
    dedupe_sources(&mut sources);
    sources.truncate(MAX_SOURCE_FILES);
    sources
}

pub(crate) fn enrich_payload(store: &WorkflowStore, run: &WorkflowRun, payload: Value) -> Value {
    let roots = source_roots(store, run);
    if let Some(path) = payload.as_str() {
        return read_source_file(&roots, path).unwrap_or_else(|| json!({"value": path}));
    }
    if let Some(path) = payload.get("path").and_then(Value::as_str)
        && payload.get("content").is_none()
        && let Some(file) = read_source_file(&roots, path)
    {
        return merge_payload(payload, file);
    }
    payload
}

pub(crate) fn effective_root(store: &WorkflowStore, run: &WorkflowRun) -> PathBuf {
    source_roots(store, run)
        .into_iter()
        .next()
        .unwrap_or_else(|| store_project_root(store))
}

pub(crate) fn implementation_root_for_targets(
    store: &WorkflowStore,
    run: &WorkflowRun,
    targets: &[String],
) -> WorkflowResult<PathBuf> {
    if let Some(root) = run
        .spec
        .target_repository_root
        .as_deref()
        .map(str::trim)
        .filter(|root| !root.is_empty())
    {
        let path = Path::new(root);
        return repository_root_for_path(path).ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "target_repository_root '{}' is not inside a Git/Cargo repository",
                path.display()
            ))
        });
    }
    repository_source_roots(store, run)
        .into_iter()
        .next()
        .or_else(|| absolute_target_parent(targets))
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(
                "implementation stage requires a target_repository_root or discoverable Git/Cargo repository path in the workflow spec".into(),
            )
        })
}

pub(crate) fn implementation_root_for_payload_targets(
    store: &WorkflowStore,
    run: &WorkflowRun,
    payload: &Value,
    targets: &[String],
) -> WorkflowResult<PathBuf> {
    if let Some(root) = declared_target_root(store, payload) {
        validate_target_root(&root, targets)?;
        return Ok(root);
    }
    if let Some(root) = project_root_for_absolute_targets(store, targets) {
        return Ok(root);
    }
    implementation_root_for_targets(store, run, targets)
}

pub(crate) fn fanout_item_target_root(
    store: &WorkflowStore,
    run: &WorkflowRun,
    payload: &Value,
    fallback_targets: &[String],
) -> PathBuf {
    let mut targets = payload_target_files(payload);
    if targets.is_empty() {
        targets = fallback_targets.to_vec();
    }
    implementation_root_for_payload_targets(store, run, payload, &targets)
        .unwrap_or_else(|_| effective_root(store, run))
}

pub(crate) fn item_targets_need_serial_root(
    store: &WorkflowStore,
    run: &WorkflowRun,
    payload: &Value,
    targets: &[String],
    canonical: &Path,
) -> bool {
    implementation_root_for_payload_targets(store, run, payload, targets)
        .is_ok_and(|root| !same_root(&root, canonical))
}

fn absolute_target_parent(targets: &[String]) -> Option<PathBuf> {
    targets
        .iter()
        .map(|target| Path::new(target.trim()))
        .filter(|target| target.is_absolute())
        .find_map(|target| target.parent().map(Path::to_path_buf))
}

fn declared_target_root(store: &WorkflowStore, payload: &Value) -> Option<PathBuf> {
    let root = payload
        .get("target_repository_root")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|root| !root.is_empty())?;
    let path = Path::new(root);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        store_project_root(store).join(path)
    })
}

fn project_root_for_absolute_targets(store: &WorkflowStore, targets: &[String]) -> Option<PathBuf> {
    let project = store_project_root(store);
    targets
        .iter()
        .map(|target| Path::new(target.trim()))
        .filter(|target| target.is_absolute())
        .any(|target| path_starts_with(target, &project))
        .then_some(project)
}

fn source_roots(store: &WorkflowStore, run: &WorkflowRun) -> Vec<PathBuf> {
    let mut roots = repository_source_roots(store, run);
    if let Some(root) = run
        .spec
        .target_repository_root
        .as_deref()
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .and_then(|root| repository_root_for_path(Path::new(root)))
    {
        roots.insert(0, root);
    }
    roots.push(store_project_root(store));
    dedupe_paths(&mut roots);
    roots
}

fn repository_source_roots(_store: &WorkflowStore, run: &WorkflowRun) -> Vec<PathBuf> {
    let mut roots = repository_roots(&run.spec.task);
    for stage in &run.spec.stages {
        if let Some(task) = &stage.task {
            roots.extend(repository_roots(task));
        }
        collect_repository_roots_from_value(&stage.input, &mut roots);
        for value in stage.extra.values() {
            collect_repository_roots_from_value(value, &mut roots);
        }
        for target in &stage.expected_target_files {
            roots.extend(repository_roots(target));
        }
    }
    dedupe_paths(&mut roots);
    roots
}

fn repository_roots(text: &str) -> Vec<PathBuf> {
    text.split_whitespace()
        .map(|part| part.trim_matches(|ch: char| matches!(ch, '.' | ',' | ':' | ';' | '"' | '\'')))
        .filter(|part| part.starts_with('/') || part.starts_with('~'))
        .map(PathBuf::from)
        .filter_map(|path| repository_root_for_path(&path))
        .collect()
}

fn repository_root_for_path(path: &Path) -> Option<PathBuf> {
    let mut cursor = if path.is_dir() {
        Some(path)
    } else {
        path.parent()
    };
    while let Some(dir) = cursor {
        if dir.join(".git").is_dir() || dir.join("Cargo.toml").is_file() {
            return Some(dir.to_path_buf());
        }
        cursor = dir.parent();
    }
    None
}

fn store_project_root(store: &WorkflowStore) -> PathBuf {
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

fn read_source_file(roots: &[PathBuf], path: &str) -> Option<Value> {
    let raw = Path::new(path);
    let candidates = if raw.is_absolute() {
        vec![raw.to_path_buf()]
    } else {
        roots.iter().map(|root| root.join(raw)).collect()
    };
    candidates
        .into_iter()
        .find_map(|path| read_existing_source(roots, &path))
}

fn read_existing_source(roots: &[PathBuf], path: &Path) -> Option<Value> {
    let canonical = path.canonicalize().ok()?;
    let root = roots
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .find(|root| canonical.starts_with(root))?;
    if !canonical.is_file() {
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
    Some(json!({
        "path": canonical.strip_prefix(&root).unwrap_or(&canonical).display().to_string(),
        "absolute_path": canonical.display().to_string(),
        "exists": true,
        "bytes": metadata.len(),
        "truncated": truncated,
        "content": String::from_utf8_lossy(&bytes).to_string(),
    }))
}

fn missing_source_file(roots: &[PathBuf], path: &str) -> Value {
    let raw = Path::new(path);
    let absolute = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        roots.first().cloned().unwrap_or_default().join(raw)
    };
    json!({
        "path": path,
        "absolute_path": absolute.display().to_string(),
        "exists": false,
        "bytes": 0,
        "truncated": false,
        "content": "",
    })
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

fn collect_repository_roots_from_value(value: &Value, out: &mut Vec<PathBuf>) {
    match value {
        Value::String(text) => out.extend(repository_roots(text)),
        Value::Array(items) => items
            .iter()
            .for_each(|item| collect_repository_roots_from_value(item, out)),
        Value::Object(map) => map
            .values()
            .for_each(|item| collect_repository_roots_from_value(item, out)),
        _ => {}
    }
}

fn collect_string_values(value: Option<&Value>, out: &mut BTreeSet<String>) {
    match value {
        Some(Value::String(text)) if !text.trim().is_empty() => {
            out.insert(text.clone());
        }
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .for_each(|text| {
                out.insert(text.to_string());
            }),
        _ => {}
    }
}

fn validate_target_root(root: &Path, targets: &[String]) -> WorkflowResult<()> {
    for target in targets {
        let target = Path::new(target.trim());
        if has_parent_component(target) {
            return Err(WorkflowError::SpecInvalid(format!(
                "target path escapes target_repository_root via '..': {}",
                target.display()
            )));
        }
        if target.is_absolute() && !path_starts_with(target, root) {
            return Err(WorkflowError::SpecInvalid(format!(
                "absolute target path outside target_repository_root: {}",
                target.display()
            )));
        }
    }
    Ok(())
}

fn payload_target_files(payload: &Value) -> Vec<String> {
    let mut targets = BTreeSet::new();
    for key in [
        "target_files",
        "expected_target_files",
        "target_file",
        "target_path",
    ] {
        collect_string_values(payload.get(key), &mut targets);
    }
    targets.into_iter().collect()
}

fn path_starts_with(path: &Path, root: &Path) -> bool {
    let path = clean_lexical(path);
    let root = clean_lexical(root);
    path.starts_with(root)
}

fn clean_lexical(path: &Path) -> PathBuf {
    path.components()
        .fold(PathBuf::new(), |mut out, component| {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    out.pop();
                }
                _ => out.push(component.as_os_str()),
            }
            out
        })
}

fn has_parent_component(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
}

fn same_root(a: &Path, b: &Path) -> bool {
    let a = a.canonicalize().unwrap_or_else(|_| clean_lexical(a));
    let b = b.canonicalize().unwrap_or_else(|_| clean_lexical(b));
    a == b
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

fn dedupe_sources(sources: &mut Vec<Value>) {
    let mut seen = BTreeSet::new();
    sources.retain(|source| {
        let key = source
            .get("absolute_path")
            .or_else(|| source.get("path"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        !key.is_empty() && seen.insert(key)
    });
}

fn dedupe_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = BTreeSet::new();
    paths.retain(|path| seen.insert(path.display().to_string()));
}
