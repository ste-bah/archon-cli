use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::{WorkflowV2ImplementationStatus, WorkflowV2TaskFileStatus, WorkflowV2TaskRecord};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2PrdIntake {
    pub prd_path: PathBuf,
    pub task_dir: PathBuf,
    pub index_paths: Vec<PathBuf>,
    pub context_paths: Vec<PathBuf>,
    pub hard_rules: Vec<String>,
    pub task_records: Vec<WorkflowV2TaskRecord>,
}

impl WorkflowV2PrdIntake {
    pub fn discover(
        prd_path: impl Into<PathBuf>,
        task_dir: impl Into<PathBuf>,
    ) -> Result<Self, WorkflowV2PrdIntakeError> {
        let prd_path = prd_path.into();
        let task_dir = task_dir.into();
        if !prd_path.is_file() {
            return Err(WorkflowV2PrdIntakeError::MissingPrd(prd_path));
        }
        if !task_dir.is_dir() {
            return Err(WorkflowV2PrdIntakeError::MissingTaskDir(task_dir));
        }

        let index_paths = index_paths(&task_dir);
        let context_paths = context_paths(&task_dir);
        let hard_rules = global_hard_rules(&prd_path, &index_paths, &context_paths)?;
        let task_files = ordered_task_files(&task_dir, &index_paths)?;
        if task_files.is_empty() {
            return Err(WorkflowV2PrdIntakeError::NoTaskFiles(task_dir));
        }

        let mut records = Vec::new();
        let mut seen_task_ids = BTreeMap::new();
        for task_file in task_files {
            let record = parse_task_file(&task_file, &hard_rules)?;
            if let Some(previous_path) =
                seen_task_ids.insert(record.task_id.clone(), task_file.clone())
            {
                return Err(WorkflowV2PrdIntakeError::DuplicateTaskId {
                    task_id: record.task_id,
                    first_path: previous_path,
                    second_path: task_file,
                });
            }
            records.push(record);
        }

        Ok(Self {
            prd_path,
            task_dir,
            index_paths,
            context_paths,
            hard_rules,
            task_records: records,
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkflowV2PrdIntakeError {
    #[error("PRD file does not exist: {0}")]
    MissingPrd(PathBuf),
    #[error("task directory does not exist: {0}")]
    MissingTaskDir(PathBuf),
    #[error("task directory contains no TASK*.md files: {0}")]
    NoTaskFiles(PathBuf),
    #[error("failed to read {path}: {message}")]
    ReadFile { path: PathBuf, message: String },
    #[error("task file {path} has no canonical task id")]
    MissingTaskId { path: PathBuf },
    #[error("duplicate canonical task id {task_id}: {first_path} and {second_path}")]
    DuplicateTaskId {
        task_id: String,
        first_path: PathBuf,
        second_path: PathBuf,
    },
}

fn index_paths(task_dir: &Path) -> Vec<PathBuf> {
    ["README.md", "INDEX.md"]
        .into_iter()
        .map(|name| task_dir.join(name))
        .filter(|path| path.is_file())
        .collect()
}

fn context_paths(task_dir: &Path) -> Vec<PathBuf> {
    let context_dir = task_dir.join("context");
    if !context_dir.is_dir() {
        return Vec::new();
    }
    let mut paths = read_dir_files(&context_dir, |path| {
        path.extension().and_then(|ext| ext.to_str()) == Some("md")
    });
    paths.sort();
    paths
}

fn global_hard_rules(
    prd_path: &Path,
    index_paths: &[PathBuf],
    context_paths: &[PathBuf],
) -> Result<Vec<String>, WorkflowV2PrdIntakeError> {
    let mut rules = BTreeSet::new();
    for path in std::iter::once(prd_path)
        .chain(index_paths.iter().map(PathBuf::as_path))
        .chain(context_paths.iter().map(PathBuf::as_path))
    {
        let body = read_to_string(path)?;
        for rule in extract_sections(&body, &["Hard Rules", "Constraints"]) {
            rules.insert(rule);
        }
    }
    Ok(rules.into_iter().collect())
}

fn ordered_task_files(
    task_dir: &Path,
    index_paths: &[PathBuf],
) -> Result<Vec<PathBuf>, WorkflowV2PrdIntakeError> {
    let mut files = read_dir_files(task_dir, |path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("TASK") && name.ends_with(".md"))
    });
    files.sort();

    let mut order = BTreeMap::new();
    let index_text = read_index_text(index_paths)?;
    for path in &files {
        if let Some(pos) = index_position(&index_text, path) {
            order.insert(path.clone(), pos);
        }
    }
    files.sort_by(|left, right| {
        let left_order = order.get(left).copied().unwrap_or(usize::MAX);
        let right_order = order.get(right).copied().unwrap_or(usize::MAX);
        left_order.cmp(&right_order).then_with(|| left.cmp(right))
    });
    Ok(files)
}

fn parse_task_file(
    path: &Path,
    global_hard_rules: &[String],
) -> Result<WorkflowV2TaskRecord, WorkflowV2PrdIntakeError> {
    let body = read_to_string(path)?;
    let task_id = task_id(path, &body)
        .ok_or_else(|| WorkflowV2PrdIntakeError::MissingTaskId { path: path.into() })?;
    let mut hard_rules = global_hard_rules.to_vec();
    hard_rules.extend(extract_sections(&body, &["Hard Rules", "Constraints"]));
    hard_rules.sort();
    hard_rules.dedup();

    Ok(WorkflowV2TaskRecord {
        task_id,
        title: title(path, &body),
        source_paths: vec![path.display().to_string()],
        depends_on: dependencies(&body),
        acceptance_criteria: acceptance_criteria(&body),
        hard_rules,
        candidate_target_files: target_files(&body),
        status_from_task_file: status_from_task_file(&body),
        implementation_status: WorkflowV2ImplementationStatus::Unknown,
    })
}

fn task_id(path: &Path, body: &str) -> Option<String> {
    line_value(body, "task_id")
        .or_else(|| filename_task_id(path))
        .and_then(|raw| canonical_task_id(&raw))
}

fn filename_task_id(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
}

fn canonical_task_id(raw: &str) -> Option<String> {
    let upper = raw.trim().trim_matches(['"', '\'']).to_ascii_uppercase();
    if let Some(pos) = upper.rfind('T') {
        let digits = upper[pos + 1..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        if !digits.is_empty() {
            return Some(format!("T{}", pad_task_digits(&digits)));
        }
    }
    let digits = upper
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    (!digits.is_empty()).then(|| format!("T{}", pad_task_digits(&digits)))
}

fn pad_task_digits(digits: &str) -> String {
    if digits.len() >= 3 {
        digits.to_string()
    } else {
        format!("{:0>3}", digits)
    }
}

fn title(path: &Path, body: &str) -> String {
    line_value(body, "title")
        .or_else(|| {
            body.lines()
                .map(str::trim)
                .find_map(|line| line.strip_prefix("# ").map(str::trim).map(str::to_string))
        })
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("task")
                .to_string()
        })
}

fn dependencies(body: &str) -> Vec<String> {
    line_value(body, "depends_on")
        .map(|value| task_id_list(&value))
        .unwrap_or_default()
}

fn acceptance_criteria(body: &str) -> Vec<String> {
    let mut criteria = extract_sections(body, &["Acceptance Criteria", "Definition of Done"]);
    criteria.extend(list_values(body, "acceptance_criteria"));
    criteria.sort();
    criteria.dedup();
    criteria
}

fn target_files(body: &str) -> Vec<String> {
    let mut targets = BTreeSet::new();
    for value in list_values(body, "target_files")
        .into_iter()
        .chain(list_values(body, "expected_target_files"))
    {
        push_target(&value, &mut targets);
    }
    for section in extract_raw_sections(
        body,
        &[
            "Files Expected to Change",
            "Target Files",
            "Required Existing Anchors",
        ],
    ) {
        collect_code_span_paths(section, &mut targets);
        collect_plain_paths(section, &mut targets);
    }
    targets.into_iter().collect()
}

fn status_from_task_file(body: &str) -> WorkflowV2TaskFileStatus {
    match line_value(body, "status")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "not_started" | "todo" | "ready" => WorkflowV2TaskFileStatus::NotStarted,
        "in_progress" | "started" => WorkflowV2TaskFileStatus::InProgress,
        "blocked" => WorkflowV2TaskFileStatus::Blocked,
        "done" | "complete" | "completed" => WorkflowV2TaskFileStatus::Done,
        _ => WorkflowV2TaskFileStatus::Unknown,
    }
}

fn extract_sections(body: &str, headings: &[&str]) -> Vec<String> {
    extract_raw_sections(body, headings)
        .into_iter()
        .flat_map(markdown_list_items)
        .collect()
}

fn extract_raw_sections<'a>(body: &'a str, headings: &[&str]) -> Vec<&'a str> {
    let mut sections = Vec::new();
    let lines = body
        .split_inclusive('\n')
        .scan(0usize, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some((start, line))
        })
        .collect::<Vec<_>>();

    let mut idx = 0usize;
    while idx < lines.len() {
        let (start, line) = lines[idx];
        let Some((level, title)) = markdown_heading(line.trim()) else {
            idx += 1;
            continue;
        };
        if !headings
            .iter()
            .any(|heading| title.eq_ignore_ascii_case(heading))
        {
            idx += 1;
            continue;
        }

        let content_start = start + line.len();
        let mut end = body.len();
        for (next_start, next_line) in lines.iter().skip(idx + 1) {
            if let Some((next_level, _)) = markdown_heading(next_line.trim()) {
                if next_level <= level {
                    end = *next_start;
                    break;
                }
            }
        }
        sections.push(body[content_start..end].trim());
        idx += 1;
    }
    sections
}

fn markdown_heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if hashes < 2 || !line.get(hashes..)?.starts_with(' ') {
        return None;
    }
    Some((hashes, line[hashes..].trim()))
}

fn markdown_list_items(section: &str) -> Vec<String> {
    section
        .lines()
        .map(|line| line.trim().trim_start_matches('-').trim())
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn line_value(body: &str, key: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let line = line.trim();
        let (found, value) = line.split_once(':')?;
        key_matches(found, key).then(|| value.trim().trim_matches(['"', '\'']).to_string())
    })
}

fn list_values(body: &str, key: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut in_list = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some((found, value)) = trimmed.split_once(':') {
            if !key_matches(found, key) {
                if in_list && !trimmed.is_empty() && !trimmed.starts_with('-') {
                    break;
                }
                continue;
            }
            in_list = true;
            values.extend(inline_list(value));
            continue;
        }
        if in_list && trimmed.starts_with('-') {
            values.push(trimmed.trim_start_matches('-').trim().to_string());
        } else if in_list && !trimmed.is_empty() {
            break;
        }
    }
    values.into_iter().filter(|v| !v.is_empty()).collect()
}

fn key_matches(found: &str, expected: &str) -> bool {
    found
        .trim()
        .replace('-', "_")
        .eq_ignore_ascii_case(expected)
}

fn inline_list(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|item| item.trim().trim_matches(['"', '\'']).to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn task_id_list(value: &str) -> Vec<String> {
    inline_list(value)
        .into_iter()
        .filter_map(|value| canonical_task_id(&value))
        .collect()
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
        ".sh", ".sql", ".go", ".java", ".kt", ".cs",
    ]
    .iter()
    .any(|ext| path.ends_with(ext))
}

fn read_index_text(paths: &[PathBuf]) -> Result<String, WorkflowV2PrdIntakeError> {
    let mut text = String::new();
    for path in paths {
        text.push_str(&read_to_string(path)?);
        text.push('\n');
    }
    Ok(text)
}

fn index_position(index_text: &str, path: &Path) -> Option<usize> {
    let name = path.file_name()?.to_str()?;
    let upper_index = index_text.to_ascii_uppercase();
    let upper_name = name.to_ascii_uppercase();
    upper_index.find(&upper_name).or_else(|| {
        filename_task_id(path)
            .and_then(|id| canonical_task_id(&id))
            .and_then(|id| upper_index.find(&id))
    })
}

fn read_dir_files(dir: &Path, predicate: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && predicate(path))
        .collect()
}

fn read_to_string(path: &Path) -> Result<String, WorkflowV2PrdIntakeError> {
    fs::read_to_string(path).map_err(|err| WorkflowV2PrdIntakeError::ReadFile {
        path: path.to_path_buf(),
        message: err.to_string(),
    })
}
