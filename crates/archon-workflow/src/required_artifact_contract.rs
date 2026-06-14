use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::required_artifacts::required_artifact_paths;
use crate::spec::{StageKind, WorkflowSpec};

const MAX_REFERENCED_FILES: usize = 16;
const MAX_REFERENCED_BYTES: usize = 256 * 1024;

pub(crate) fn ensure_final_required_artifacts(spec: &mut WorkflowSpec) {
    let inferred = infer_required_artifacts(spec);
    if inferred.is_empty() {
        return;
    }
    let Some(gate) = spec
        .stages
        .iter_mut()
        .rfind(|stage| stage.kind == StageKind::QualityGate)
    else {
        return;
    };
    let mut paths = required_artifact_paths(gate);
    paths.extend(inferred);
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return;
    }
    gate.extra.insert(
        "required_artifacts".into(),
        Value::Array(paths.into_iter().map(Value::String).collect()),
    );
    gate.extra
        .insert("required_artifacts_inferred".into(), Value::Bool(true));
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
    let mut files = BTreeSet::new();
    for text in texts {
        for token in text.split_whitespace().map(clean_token) {
            if !looks_like_reference(&token) {
                continue;
            }
            let path = PathBuf::from(&token);
            if path.is_file() {
                files.insert(path);
            } else if path.is_dir() {
                for child in markdown_files(&path) {
                    files.insert(child);
                    if files.len() >= MAX_REFERENCED_FILES {
                        return files.into_iter().collect();
                    }
                }
            }
            if files.len() >= MAX_REFERENCED_FILES {
                return files.into_iter().collect();
            }
        }
    }
    files.into_iter().collect()
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
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| matches!(ext, "md" | "yaml" | "yml" | "json"))
        })
        .collect::<Vec<_>>();
    files.sort();
    files.truncate(MAX_REFERENCED_FILES);
    files
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
        if concrete_project_artifact(&token) {
            out.insert(token);
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
        if line.ends_with('/') && !line.contains('<') {
            if let Some(parent) = stack.last().map(|(_, path)| path.clone()) {
                stack.push((indent, join_path(&parent, line.trim_end_matches('/'))));
            }
        } else if artifact_leaf(&line)
            && let Some(parent) = stack.last().map(|(_, path)| path.clone())
        {
            out.insert(join_path(&parent, &line));
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
    } else if concrete_project_artifact(&cleaned) {
        out.insert(cleaned);
    }
}

fn artifact_leaf(line: &str) -> bool {
    !line.contains('<') && !line.contains('>') && artifact_extension(line)
}

fn concrete_project_artifact(path: &str) -> bool {
    path.starts_with(".archon/")
        && !path.contains('<')
        && !path.contains('>')
        && artifact_extension(path)
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
