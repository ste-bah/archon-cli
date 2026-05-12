use std::{
    fs,
    path::{Path, PathBuf},
};

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{AppState, check_auth, inspect::PathProbe};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PipelineSummary {
    pub definitions: Vec<PathProbe>,
    pub session_count: u64,
    pub recent_sessions: Vec<String>,
    pub artifact_roots: Vec<PathProbe>,
    pub stages: Vec<PipelineStageSummary>,
    pub agents: Vec<PipelineAgentSummary>,
    pub runs: Vec<PipelineRunSummary>,
    pub outputs: Vec<PipelineOutputSummary>,
    pub live_events: Vec<PipelineLiveEventPreview>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PipelineStageSummary {
    pub label: String,
    pub family: String,
    pub status: String,
    pub agent_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PipelineAgentSummary {
    pub name: String,
    pub family: String,
    pub responsibility: String,
    pub path: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PipelineRunSummary {
    pub run_id: String,
    pub family: String,
    pub status: String,
    pub path: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PipelineOutputSummary {
    pub label: String,
    pub kind: String,
    pub path: String,
    pub bytes: u64,
    pub updated_at: String,
    pub tail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PipelineLiveEventPreview {
    pub session_id: String,
    pub event_type: String,
    pub status: String,
    pub summary: String,
    pub path: String,
}

pub(crate) async fn summary_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    (StatusCode::OK, Json(pipeline_summary())).into_response()
}

fn pipeline_summary() -> PipelineSummary {
    let cwd = cwd();
    let home = home_archon();
    let sessions = home.join("sessions");
    let definitions = pipeline_definitions(&cwd);
    PipelineSummary {
        session_count: count_dirs(&sessions),
        recent_sessions: recent_dir_names(&sessions, 12),
        artifact_roots: artifact_roots(&cwd, &home),
        stages: stage_summaries(&cwd),
        agents: agent_summaries(&cwd, 18),
        runs: run_summaries(&home, 10),
        outputs: output_summaries(&cwd, &home, 12),
        live_events: live_events(&home, 10),
        definitions,
    }
}

fn pipeline_definitions(cwd: &Path) -> Vec<PathProbe> {
    vec![
        probe(
            "coding pipeline",
            cwd.join(".archon/agents/coding-pipeline/pipeline.toml"),
        ),
        probe("gametheory agents", cwd.join(".archon/agents/gametheory")),
        probe("research agents", cwd.join(".archon/agents/phdresearch")),
    ]
}

fn artifact_roots(cwd: &Path, home: &Path) -> Vec<PathProbe> {
    vec![
        probe("local artifacts", cwd.join(".archon/artifacts")),
        probe("pipeline bundles", home.join("pipelines")),
    ]
}

fn stage_summaries(cwd: &Path) -> Vec<PipelineStageSummary> {
    vec![
        stage(
            "Intake",
            "coding",
            "ready",
            agent_count(cwd, "coding-pipeline", 0, 9),
        ),
        stage(
            "Design",
            "coding",
            "ready",
            agent_count(cwd, "coding-pipeline", 10, 24),
        ),
        stage(
            "Implementation",
            "coding",
            "ready",
            agent_count(cwd, "coding-pipeline", 25, 37),
        ),
        stage(
            "Verification",
            "coding",
            "ready",
            agent_count(cwd, "coding-pipeline", 38, 47),
        ),
        stage(
            "Research",
            "research",
            status_for(cwd, "phdresearch"),
            count_agents(cwd, "phdresearch"),
        ),
        stage(
            "Game theory",
            "gametheory",
            status_for(cwd, "gametheory"),
            count_agents(cwd, "gametheory"),
        ),
    ]
}

fn agent_summaries(cwd: &Path, limit: usize) -> Vec<PipelineAgentSummary> {
    let families = [
        ("coding", "coding-pipeline"),
        ("research", "phdresearch"),
        ("gametheory", "gametheory"),
    ];
    families
        .iter()
        .flat_map(|(family, dir)| list_agent_files(cwd, family, dir))
        .take(limit)
        .collect()
}

fn list_agent_files(cwd: &Path, family: &str, dir: &str) -> Vec<PipelineAgentSummary> {
    let root = cwd.join(".archon/agents").join(dir);
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut agents: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                return None;
            }
            let name = path.file_stem()?.to_string_lossy().replace('-', " ");
            Some(PipelineAgentSummary {
                name,
                family: family.into(),
                responsibility: first_non_empty_line(&path)
                    .unwrap_or_else(|| "agent prompt".into()),
                path: display_path(&path),
                status: "ready".into(),
            })
        })
        .collect();
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    agents
}

fn run_summaries(home: &Path, limit: usize) -> Vec<PipelineRunSummary> {
    let root = home.join("pipelines");
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut runs: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_dir() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            Some(PipelineRunSummary {
                family: infer_family(&name),
                status: infer_run_status(&entry.path()),
                updated_at: modified_label(&metadata),
                path: display_path(&entry.path()),
                run_id: name,
            })
        })
        .collect();
    runs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    runs.into_iter().take(limit).collect()
}

fn output_summaries(cwd: &Path, home: &Path, limit: usize) -> Vec<PipelineOutputSummary> {
    let roots = [cwd.join(".archon/artifacts"), home.join("pipelines")];
    let mut outputs: Vec<_> = roots
        .iter()
        .flat_map(|root| recent_files(root, 2, limit))
        .collect();
    outputs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    outputs.into_iter().take(limit).collect()
}

fn recent_files(root: &Path, depth: usize, limit: usize) -> Vec<PipelineOutputSummary> {
    if depth == 0 {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten().take(limit * 3) {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            out.extend(recent_files(&path, depth - 1, limit));
        } else {
            out.push(PipelineOutputSummary {
                label: entry.file_name().to_string_lossy().to_string(),
                kind: path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("file")
                    .into(),
                path: display_path(&path),
                bytes: metadata.len(),
                updated_at: modified_label(&metadata),
                tail: latest_line(&path).unwrap_or_else(|| "binary or empty artifact".into()),
            });
        }
    }
    out
}

fn live_events(home: &Path, limit: usize) -> Vec<PipelineLiveEventPreview> {
    let root = home.join("sessions");
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut events: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.contains("activity")
                || path.extension().and_then(|ext| ext.to_str()) != Some("jsonl")
            {
                return None;
            }
            let summary = latest_line(&path)?;
            Some(PipelineLiveEventPreview {
                session_id: name.replace(".activity.jsonl", ""),
                event_type: if summary.contains("pipeline") {
                    "pipeline"
                } else {
                    "activity"
                }
                .into(),
                status: if summary.contains("failed") {
                    "warn"
                } else {
                    "recorded"
                }
                .into(),
                summary,
                path: display_path(&path),
            })
        })
        .collect();
    events.sort_by(|a, b| b.session_id.cmp(&a.session_id));
    events.into_iter().take(limit).collect()
}

fn stage(label: &str, family: &str, status: &str, agent_count: u64) -> PipelineStageSummary {
    PipelineStageSummary {
        label: label.into(),
        family: family.into(),
        status: status.into(),
        agent_count,
    }
}

fn status_for(cwd: &Path, dir: &str) -> &'static str {
    if cwd.join(".archon/agents").join(dir).exists() {
        "ready"
    } else {
        "missing"
    }
}

fn count_agents(cwd: &Path, dir: &str) -> u64 {
    fs::read_dir(cwd.join(".archon/agents").join(dir))
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
        .count() as u64
}

fn agent_count(cwd: &Path, dir: &str, start: u64, end: u64) -> u64 {
    let total = count_agents(cwd, dir);
    total.min(end.saturating_sub(start) + 1)
}

fn first_non_empty_line(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.chars().take(140).collect())
}

fn latest_line(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()?
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.chars().take(180).collect())
}

fn infer_family(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.contains("research") {
        "research".into()
    } else if lower.contains("game") {
        "gametheory".into()
    } else {
        "coding".into()
    }
}

fn infer_run_status(path: &Path) -> String {
    if path.join("complete.json").exists() || path.join("report.md").exists() {
        "completed".into()
    } else if path.join("failed.json").exists() {
        "failed".into()
    } else {
        "recorded".into()
    }
}

fn recent_dir_names(root: &Path, limit: usize) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_dir().then_some((
                modified_label(&metadata),
                entry.file_name().to_string_lossy().to_string(),
            ))
        })
        .collect();
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    dirs.into_iter().take(limit).map(|(_, name)| name).collect()
}

fn count_dirs(path: &Path) -> u64 {
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .count() as u64
}

fn probe(label: impl Into<String>, path: PathBuf) -> PathProbe {
    let (files, bytes) = dir_stats(&path, 0);
    PathProbe {
        label: label.into(),
        path: display_path(&path),
        exists: path.exists(),
        files,
        bytes,
    }
}

fn dir_stats(path: &Path, depth: usize) -> (u64, u64) {
    if depth > 3 {
        return (0, 0);
    }
    let Ok(metadata) = fs::metadata(path) else {
        return (0, 0);
    };
    if metadata.is_file() {
        return (1, metadata.len());
    }
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .fold((0, 0), |(files, bytes), entry| {
            let (child_files, child_bytes) = dir_stats(&entry.path(), depth + 1);
            (files + child_files, bytes + child_bytes)
        })
}

fn modified_label(metadata: &fs::Metadata) -> String {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|| "0".into())
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn home_archon() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".archon")
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(PipelineSummary::decl(&cfg)),
        exported(PipelineStageSummary::decl(&cfg)),
        exported(PipelineAgentSummary::decl(&cfg)),
        exported(PipelineRunSummary::decl(&cfg)),
        exported(PipelineOutputSummary::decl(&cfg)),
        exported(PipelineLiveEventPreview::decl(&cfg)),
    ]
    .join("\n\n")
        + "\n"
}

fn exported(decl: String) -> String {
    format!("export {decl}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_pipeline_root_returns_empty_runs() {
        let runs = run_summaries(Path::new("/definitely/missing/archon-home"), 4);
        assert!(runs.is_empty());
    }
}
