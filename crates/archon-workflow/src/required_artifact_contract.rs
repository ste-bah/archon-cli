use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::required_artifacts::required_artifact_paths;
use crate::spec::{StageKind, WorkflowSpec};

const MAX_REFERENCED_FILES: usize = 256;
const MAX_REFERENCED_BYTES: usize = 1024 * 1024;

pub(crate) fn ensure_final_required_artifacts(spec: &mut WorkflowSpec) {
    let inferred = infer_required_artifacts(spec);
    let Some(gate) = spec
        .stages
        .iter_mut()
        .rfind(|stage| stage.kind == StageKind::QualityGate)
    else {
        return;
    };
    if inferred.is_empty() {
        return;
    }
    let hard_required = required_artifact_paths(gate);
    gate.extra
        .entry("workflow_contracts".into())
        .or_insert_with(|| contract_report(&hard_required, &inferred));
    gate.extra
        .insert("workflow_contracts_inferred".into(), Value::Bool(true));
    gate.extra
        .insert("required_artifacts_inferred".into(), Value::Bool(false));
}

fn contract_report(hard_required: &[String], candidates: &[String]) -> Value {
    serde_json::json!({
        "schema": "archon.workflow.contracts.v1",
        "hard_required_artifacts": hard_required,
        "candidate_artifacts": candidates,
        "enforcement": {
            "hard_required_artifacts": "Only artifacts explicitly declared in the WorkflowSpec are enforced by required_artifacts gates.",
            "candidate_artifacts": "Paths inferred from task, PRD, or markdown layout text are advisory candidates until a planner or workflow author promotes them explicitly."
        }
    })
}

fn infer_required_artifacts(spec: &WorkflowSpec) -> Vec<String> {
    let mut texts = seed_texts(spec);
    for path in referenced_files(&texts) {
        if let Some(text) = read_reference(&path) {
            texts.push(text);
        }
    }
    let mut paths = BTreeSet::new();
    for text in texts {
        collect_inline_project_paths(&text, &mut paths);
        collect_layout_paths(&text, &mut paths);
    }
    paths.into_iter().collect()
}

fn seed_texts(spec: &WorkflowSpec) -> Vec<String> {
    let mut texts = vec![spec.task.clone()];
    for stage in &spec.stages {
        if let Some(task) = &stage.task {
            texts.push(task.clone());
        }
        collect_json_strings(&stage.input, &mut texts);
        for value in stage.extra.values() {
            collect_json_strings(value, &mut texts);
        }
    }
    texts
}

fn collect_json_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(values) => values
            .iter()
            .for_each(|value| collect_json_strings(value, out)),
        Value::Object(map) => map
            .values()
            .for_each(|value| collect_json_strings(value, out)),
        _ => {}
    }
}

fn referenced_files(texts: &[String]) -> Vec<PathBuf> {
    let mut explicit_files = BTreeSet::new();
    let mut dirs = BTreeSet::new();
    for text in texts {
        for token in text.split_whitespace().map(clean_token) {
            if !looks_like_reference(&token) {
                continue;
            }
            let path = PathBuf::from(&token);
            if path.is_file() {
                explicit_files.insert(path);
            } else if path.is_dir() {
                dirs.insert(path);
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for file in explicit_files {
        push_reference_file(file, &mut seen, &mut ordered);
        if ordered.len() >= MAX_REFERENCED_FILES {
            return ordered;
        }
    }
    for dir in dirs {
        for child in markdown_files(&dir) {
            push_reference_file(child, &mut seen, &mut ordered);
            if ordered.len() >= MAX_REFERENCED_FILES {
                return ordered;
            }
        }
    }
    ordered
}

fn clean_token(token: &str) -> String {
    token
        .trim_start_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | '`' | ',' | ';' | ':' | ')' | '(' | '[' | ']'
            )
        })
        .trim_end_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | '`' | ',' | ';' | ':' | ')' | '(' | '[' | ']' | '.'
            )
        })
        .to_string()
}

fn looks_like_reference(token: &str) -> bool {
    let path = Path::new(token);
    (path.is_absolute() || token.starts_with("~/"))
        && (token.ends_with(".md")
            || token.ends_with(".yaml")
            || token.ends_with(".yml")
            || token.ends_with(".json")
            || path.is_dir())
}

fn markdown_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_markdown_files(dir, &mut files);
    files.sort();
    files.truncate(MAX_REFERENCED_FILES);
    files
}

fn collect_markdown_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries = entries
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if files.len() >= MAX_REFERENCED_FILES {
            return;
        }
        if path.is_dir() {
            collect_markdown_files(&path, files);
        } else if is_reference_file(&path) {
            files.push(path);
        }
    }
}

fn push_reference_file(path: PathBuf, seen: &mut BTreeSet<PathBuf>, ordered: &mut Vec<PathBuf>) {
    if seen.insert(path.clone()) {
        ordered.push(path);
    }
}

fn is_reference_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| matches!(ext, "md" | "yaml" | "yml" | "json"))
}

fn read_reference(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() as usize > MAX_REFERENCED_BYTES {
        return None;
    }
    fs::read_to_string(path).ok()
}

fn collect_inline_project_paths(text: &str, out: &mut BTreeSet<String>) {
    for token in text.split_whitespace().map(clean_token) {
        if let Some(path) = project_artifact_pattern(&token) {
            out.insert(path);
        }
    }
}

fn collect_layout_paths(text: &str, out: &mut BTreeSet<String>) {
    let mut stack: Vec<(usize, String)> = Vec::new();
    for raw in text.lines() {
        let indent = raw.chars().take_while(|ch| ch.is_whitespace()).count();
        let line = raw.trim();
        let line = clean_layout_line(line);
        if line.is_empty() || line.starts_with('#') || line.starts_with("```") {
            continue;
        }
        if line.starts_with(".archon/") {
            handle_root_line(indent, &line, &mut stack, out);
            continue;
        }
        if stack.is_empty() {
            continue;
        }
        while stack.last().is_some_and(|(level, _)| indent <= *level) {
            stack.pop();
        }
        if line.ends_with('/') {
            if let Some(parent) = stack.last().map(|(_, path)| path.clone()) {
                let child = layout_dir_segment(line.trim_end_matches('/'));
                stack.push((indent, join_path(&parent, &child)));
            }
        } else if let Some(parent) = stack.last().map(|(_, path)| path.clone()) {
            let path = join_path(&parent, &line);
            if let Some(pattern) = project_artifact_pattern(&path) {
                out.insert(pattern);
            }
        }
    }
}

fn clean_layout_line(line: &str) -> String {
    line.trim_start_matches("- ")
        .trim()
        .trim_matches('`')
        .trim()
        .to_string()
}

fn handle_root_line(
    indent: usize,
    line: &str,
    stack: &mut Vec<(usize, String)>,
    out: &mut BTreeSet<String>,
) {
    let cleaned = line.trim_end_matches('/').to_string();
    stack.clear();
    if line.ends_with('/') {
        stack.push((indent, cleaned));
    } else if let Some(pattern) = project_artifact_pattern(&cleaned) {
        out.insert(pattern);
    }
}

fn layout_dir_segment(segment: &str) -> String {
    if segment.starts_with('<') && segment.ends_with('>') {
        "*".to_string()
    } else {
        segment.to_string()
    }
}

fn project_artifact_pattern(path: &str) -> Option<String> {
    if concrete_project_artifact(path) {
        return Some(path.to_string());
    }
    dynamic_project_artifact_glob(path)
}

fn concrete_project_artifact(path: &str) -> bool {
    is_project_artifact_path(path)
        && !path.contains('<')
        && !path.contains('>')
        && artifact_extension(path)
}

fn dynamic_project_artifact_glob(path: &str) -> Option<String> {
    if !is_project_artifact_path(path) || !artifact_extension(path) {
        return None;
    }
    let globbed = replace_placeholder_segments(path)?;
    if globbed.contains('<') || globbed.contains('>') {
        return None;
    }
    Some(globbed)
}

fn is_project_artifact_path(path: &str) -> bool {
    path.starts_with(".archon/") || path.contains("/.archon/")
}

fn replace_placeholder_segments(path: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = path.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '<' {
            out.push(ch);
            continue;
        }
        let mut closed = false;
        for inner in chars.by_ref() {
            if inner == '>' {
                out.push('*');
                closed = true;
                break;
            }
        }
        if !closed {
            return None;
        }
    }
    (!out.contains('<')).then_some(out)
}

fn artifact_extension(text: &str) -> bool {
    [
        ".csv", ".db", ".duckdb", ".json", ".jsonl", ".md", ".parquet", ".pine", ".sqlite",
        ".toml", ".txt", ".yaml", ".yml",
    ]
    .iter()
    .any(|ext| text.ends_with(ext))
}

fn join_path(parent: &str, child: &str) -> String {
    format!(
        "{}/{}",
        parent.trim_end_matches('/'),
        child.trim_start_matches('/')
    )
}
