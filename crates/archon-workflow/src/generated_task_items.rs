use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::spec::StageSpec;

pub(super) fn implementation_items(spec_task: &str, stage: &StageSpec) -> Option<Vec<Value>> {
    let task_ids = referenced_task_numbers(stage);
    if task_ids.is_empty() {
        return None;
    }
    let task_files = task_files_for_ids(spec_task, &task_ids);
    let items = task_files
        .into_iter()
        .filter_map(|task_file| item_from_task_file(stage, &task_file))
        .collect::<Vec<_>>();
    (!items.is_empty()).then_some(items)
}

fn referenced_task_numbers(stage: &StageSpec) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    collect_t_numbers(&stage.id, &mut ids);
    if let Some(task) = &stage.task {
        collect_t_numbers(task, &mut ids);
        collect_task_suffixes(task, &mut ids);
    }
    ids
}

fn collect_t_numbers(text: &str, out: &mut BTreeSet<String>) {
    let chars = text.chars().collect::<Vec<_>>();
    for idx in 0..chars.len().saturating_sub(3) {
        if !matches!(chars[idx], 'T' | 't') {
            continue;
        }
        let digits = &chars[idx + 1..idx + 4];
        if digits.iter().all(char::is_ascii_digit) {
            out.insert(digits.iter().collect());
        }
    }
}

fn collect_task_suffixes(text: &str, out: &mut BTreeSet<String>) {
    for token in text.split(|ch: char| ch.is_whitespace() || ",.;:()[]{}".contains(ch)) {
        if !token.to_ascii_uppercase().starts_with("TASK-") {
            continue;
        }
        let digits = token
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>();
        if digits.len() >= 3 {
            out.insert(digits[digits.len() - 3..].to_string());
        }
    }
}

fn task_files_for_ids(spec_task: &str, task_ids: &BTreeSet<String>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in task_dirs(spec_task) {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|v| v.to_str()) != Some("md") {
                continue;
            }
            let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            if name.starts_with("TASK-") && task_ids.iter().any(|id| name.contains(id)) {
                files.push(path);
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

fn task_dirs(spec_task: &str) -> Vec<PathBuf> {
    let mut dirs = BTreeSet::new();
    for token in path_tokens(spec_task) {
        let path = PathBuf::from(token);
        if path.is_dir() {
            dirs.insert(path);
            continue;
        }
        if let Some(parent) = path.parent()
            && parent.is_dir()
        {
            dirs.insert(parent.to_path_buf());
        }
    }
    dirs.into_iter().collect()
}

fn path_tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|part| {
            part.trim_matches(|ch: char| matches!(ch, '.' | ',' | ':' | ';' | '"' | '\'' | '`'))
        })
        .filter(|part| part.starts_with('/') && part.contains("/tasks/"))
        .map(str::to_string)
        .collect()
}

fn item_from_task_file(stage: &StageSpec, task_file: &Path) -> Option<Value> {
    let body = fs::read_to_string(task_file).ok()?;
    let targets = target_files(&body);
    if targets.is_empty() {
        return None;
    }
    let task_id = task_id_from_file(task_file, &body);
    Some(json!({
        "task_id": task_id,
        "task_file": task_file.display().to_string(),
        "task": implementation_item_task(stage, &task_id, task_file),
        "target_files": targets,
        "required_tests": focused_tests(&body),
    }))
}

fn task_id_from_file(path: &Path, body: &str) -> String {
    body.lines()
        .find_map(|line| line.trim().strip_prefix("task_id:").map(str::trim))
        .map(|value| value.trim_matches(['"', '\'']).to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            path.file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "task".into())
}

fn implementation_item_task(stage: &StageSpec, task_id: &str, task_file: &Path) -> String {
    format!(
        "{}\n\nTask evidence: `{}`. Implement only missing work for `{}` and follow that task file's scope, forbidden-file, test, and residual-gap rules.",
        stage.task.as_deref().unwrap_or("Implement missing work."),
        task_file.display(),
        task_id
    )
}

fn target_files(body: &str) -> Vec<String> {
    let section = section(body, "## Files Expected to Change")
        .or_else(|| section(body, "## Required Existing Anchors"))
        .unwrap_or(body);
    let mut targets = BTreeSet::new();
    collect_code_span_paths(section, &mut targets);
    collect_plain_paths(section, &mut targets);
    targets.into_iter().collect()
}

fn focused_tests(body: &str) -> Vec<String> {
    section(body, "## Focused Tests")
        .unwrap_or_default()
        .lines()
        .map(|line| line.trim().trim_start_matches('-').trim())
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn section<'a>(body: &'a str, heading: &str) -> Option<&'a str> {
    let start = body.find(heading)?;
    let rest = &body[start + heading.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    Some(rest[..end].trim())
}

fn collect_code_span_paths(text: &str, out: &mut BTreeSet<String>) {
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('`') else {
            break;
        };
        push_target(rest[..end].trim(), out);
        rest = &rest[end + 1..];
    }
}

fn collect_plain_paths(text: &str, out: &mut BTreeSet<String>) {
    for token in text.split_whitespace() {
        push_target(
            token.trim_matches(|ch: char| matches!(ch, ',' | '.' | ':' | ';' | ')' | '(')),
            out,
        );
    }
}

fn push_target(candidate: &str, out: &mut BTreeSet<String>) {
    if candidate.starts_with("http") || candidate.starts_with('/') {
        return;
    }
    if !candidate.contains('/') || !has_source_extension(candidate) {
        return;
    }
    out.insert(candidate.to_string());
}

fn has_source_extension(path: &str) -> bool {
    [
        ".rs", ".toml", ".json", ".yaml", ".yml", ".md", ".ts", ".tsx", ".js", ".jsx", ".py",
        ".sh", ".sql",
    ]
    .iter()
    .any(|ext| path.ends_with(ext))
}
