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
use serde_json::Value;
use ts_rs::{Config as TsConfig, TS};

use super::{AppState, check_auth, inspect::PathProbe};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorldInspectionSummary {
    pub root: PathProbe,
    pub ledgers: Vec<PathProbe>,
    pub db_present: bool,
    pub candidate_count: u64,
    pub reasoning_root_present: bool,
    pub artifacts: Vec<WorldModelArtifact>,
    pub advisor_events: Vec<WorldAdvisorEventPreview>,
    pub signals: Vec<WorldModelSignal>,
    pub candidates: Vec<WorldModelRowPreview>,
    pub reasoning_events: Vec<WorldModelRowPreview>,
    pub shadow_reports: Vec<WorldModelRowPreview>,
    pub predictions: Vec<WorldPredictionPreview>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorldModelArtifact {
    pub label: String,
    pub kind: String,
    pub status: String,
    pub path: String,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorldAdvisorEventPreview {
    pub surface: String,
    pub reason: String,
    pub action_summary: String,
    pub session_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorldModelSignal {
    pub label: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorldModelRowPreview {
    pub label: String,
    pub kind: String,
    pub status: String,
    pub detail: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorldPredictionPreview {
    pub label: String,
    pub surface: String,
    pub status: String,
    pub detail: String,
    pub session_id: String,
}

pub(crate) async fn summary_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    (StatusCode::OK, Json(world_summary())).into_response()
}

fn world_summary() -> WorldInspectionSummary {
    let home = home_archon();
    let root = home.join("world-model");
    let advisor_ledger = root.join("ledgers/world-advisor-events.jsonl");
    WorldInspectionSummary {
        root: probe("world model", root.clone()),
        ledgers: vec![probe("advisor events", advisor_ledger.clone())],
        db_present: root.join("world-model.db").exists(),
        candidate_count: count_entries(&root.join("candidates")),
        reasoning_root_present: home.join("reasoning-quality").exists(),
        artifacts: vec![
            artifact("database", "cozo", root.join("world-model.db")),
            artifact("candidate registry", "candidate", root.join("candidates")),
            artifact("active model", "active", root.join("active")),
            artifact("advisor ledger", "jsonl", advisor_ledger.clone()),
            artifact(
                "reasoning bridge",
                "reasoning",
                home.join("reasoning-quality"),
            ),
        ],
        advisor_events: advisor_events(&advisor_ledger, 8),
        signals: world_signals(&root, &home, &advisor_ledger),
        candidates: row_previews(&root.join("candidates"), "candidate", 8),
        reasoning_events: row_previews(&home.join("reasoning-quality"), "reasoning", 8),
        shadow_reports: shadow_rows(&home.join("reasoning-quality"), 8),
        predictions: prediction_rows(&advisor_ledger, 8),
    }
}

fn world_signals(root: &Path, home: &Path, ledger: &Path) -> Vec<WorldModelSignal> {
    vec![
        signal("Cold start gate", "advisory", cold_start_detail(ledger)),
        signal(
            "Candidate store",
            exists_status(&root.join("candidates")),
            "world-model/candidates",
        ),
        signal(
            "Active pointer",
            exists_status(&root.join("active")),
            "world-model/active",
        ),
        signal(
            "Reasoning-quality bridge",
            exists_status(&home.join("reasoning-quality")),
            "feeds trace rows when enabled",
        ),
    ]
}

fn signal(label: &str, status: impl Into<String>, detail: impl Into<String>) -> WorldModelSignal {
    WorldModelSignal {
        label: label.into(),
        status: status.into(),
        detail: detail.into(),
    }
}

fn cold_start_detail(ledger: &Path) -> String {
    let recent = advisor_events(ledger, 20);
    let cold = recent
        .iter()
        .filter(|event| event.reason == "cold_start")
        .count();
    if cold > 0 {
        format!("{cold} recent advisor calls returned cold_start")
    } else {
        "no recent cold_start advisor events found".into()
    }
}

fn advisor_events(path: &Path, limit: usize) -> Vec<WorldAdvisorEventPreview> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .rev()
        .take(limit)
        .filter_map(parse_advisor_event)
        .collect()
}

fn parse_advisor_event(line: &str) -> Option<WorldAdvisorEventPreview> {
    let value: Value = serde_json::from_str(line).ok()?;
    let unavailable = value.get("unavailable");
    Some(WorldAdvisorEventPreview {
        surface: str_field(&value, "surface"),
        reason: unavailable
            .and_then(|entry| entry.get("reason"))
            .and_then(Value::as_str)
            .unwrap_or("available")
            .to_string(),
        action_summary: str_field(&value, "action_summary"),
        session_id: str_field(&value, "session_id"),
        created_at: str_field(&value, "created_at"),
    })
}

fn prediction_rows(path: &Path, limit: usize) -> Vec<WorldPredictionPreview> {
    advisor_events(path, limit)
        .into_iter()
        .map(|event| WorldPredictionPreview {
            label: if event.reason == "available" {
                "advisor prediction".into()
            } else {
                "advisor unavailable".into()
            },
            surface: event.surface,
            status: event.reason,
            detail: event.action_summary,
            session_id: event.session_id,
        })
        .collect()
}

fn shadow_rows(root: &Path, limit: usize) -> Vec<WorldModelRowPreview> {
    [root.join("shadow"), root.join("reports")]
        .iter()
        .flat_map(|path| row_previews(path, "shadow", limit))
        .take(limit)
        .collect()
}

fn row_previews(root: &Path, kind: &str, limit: usize) -> Vec<WorldModelRowPreview> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut rows: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            if metadata.is_dir() {
                return Some(nested_rows(&path, kind, limit));
            }
            Some(vec![file_row(&path, &metadata, kind)])
        })
        .flatten()
        .collect();
    rows.sort_by(|a, b| b.status.cmp(&a.status));
    rows.into_iter().take(limit).collect()
}

fn nested_rows(root: &Path, kind: &str, limit: usize) -> Vec<WorldModelRowPreview> {
    fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata
                .is_file()
                .then(|| file_row(&entry.path(), &metadata, kind))
        })
        .take(limit)
        .collect()
}

fn file_row(path: &Path, metadata: &fs::Metadata, kind: &str) -> WorldModelRowPreview {
    WorldModelRowPreview {
        label: path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| kind.into()),
        kind: kind.into(),
        status: metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs().to_string())
            .unwrap_or_else(|| "0".into()),
        detail: latest_line(path).unwrap_or_else(|| format!("{} bytes", metadata.len())),
        path: path.to_string_lossy().to_string(),
    }
}

fn latest_line(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()?
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.chars().take(180).collect())
}

fn str_field(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn artifact(label: &str, kind: &str, path: PathBuf) -> WorldModelArtifact {
    let (files, bytes) = dir_stats(&path, 0);
    WorldModelArtifact {
        label: label.into(),
        kind: kind.into(),
        status: exists_status(&path),
        path: path.to_string_lossy().to_string(),
        files,
        bytes,
    }
}

fn exists_status(path: &Path) -> String {
    if path.exists() { "present" } else { "missing" }.into()
}

fn probe(label: impl Into<String>, path: PathBuf) -> PathProbe {
    let (files, bytes) = dir_stats(&path, 0);
    PathProbe {
        label: label.into(),
        path: path.to_string_lossy().to_string(),
        exists: path.exists(),
        files,
        bytes,
    }
}

fn count_entries(path: &Path) -> u64 {
    if path.is_file() {
        return 1;
    }
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .count() as u64
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
        .map(|entry| dir_stats(&entry.path(), depth + 1))
        .fold((0, 0), |(files, bytes), (child_files, child_bytes)| {
            (files + child_files, bytes + child_bytes)
        })
}

fn home_archon() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".archon")
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(WorldInspectionSummary::decl(&cfg)),
        exported(WorldModelArtifact::decl(&cfg)),
        exported(WorldAdvisorEventPreview::decl(&cfg)),
        exported(WorldModelSignal::decl(&cfg)),
        exported(WorldModelRowPreview::decl(&cfg)),
        exported(WorldPredictionPreview::decl(&cfg)),
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
    fn parses_cold_start_advisor_event() {
        let event = parse_advisor_event(
            r#"{"surface":"provider_runtime","unavailable":{"reason":"cold_start"},"session_id":"s","action_summary":"stream","created_at":"now"}"#,
        )
        .unwrap();
        assert_eq!(event.reason, "cold_start");
    }
}
