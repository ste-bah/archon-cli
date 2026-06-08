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
use cozo::ScriptMutability;
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{AppState, WebRuntimePaths, check_auth};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PathProbe {
    pub label: String,
    pub path: String,
    pub exists: bool,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct LearningSummary {
    pub stores: Vec<PathProbe>,
    pub signals: Vec<LearningSignalItem>,
    pub memories: Vec<LearningRowPreview>,
    pub learning_events: Vec<LearningRowPreview>,
    pub proposals: Vec<LearningRowPreview>,
    pub trust_deltas: Vec<LearningRowPreview>,
    pub recent_sessions: Vec<String>,
    pub session_count: u64,
    pub reasoning_store_present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct LearningSignalItem {
    pub label: String,
    pub kind: String,
    pub status: String,
    pub count: u64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct LearningRowPreview {
    pub label: String,
    pub kind: String,
    pub status: String,
    pub detail: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SettingsSummary {
    pub theme_modes: Vec<String>,
    pub density_modes: Vec<String>,
    pub policy_editing_enabled: bool,
    pub direct_filesystem_open_enabled: bool,
}

pub(crate) async fn learning_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    authed_json(&state, &headers, learning_summary(&state.paths))
}

pub(crate) async fn settings_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    authed_json(&state, &headers, settings_summary())
}

fn authed_json<T: Serialize>(state: &AppState, headers: &HeaderMap, value: T) -> Response {
    if let Err(resp) = check_auth(state, headers) {
        return resp;
    }
    (StatusCode::OK, Json(value)).into_response()
}

fn learning_summary(paths: &WebRuntimePaths) -> LearningSummary {
    let sessions = paths.session_activity_root.clone();
    let recent_sessions = recent_dir_names(&sessions, 12);
    LearningSummary {
        stores: vec![
            probe(
                "local learning-state db",
                paths.cwd.join(".archon/learning-state.db"),
            ),
            probe(
                "legacy local learning db",
                paths.cwd.join(".archon/learning.db"),
            ),
            probe("session activity", sessions),
            probe("memory database", paths.memory_db.clone()),
            probe("reasoning quality", paths.reasoning_quality_root.clone()),
        ],
        signals: learning_signals(paths),
        memories: memory_rows(&paths.memory_db, 8),
        learning_events: ledger_rows(&paths.archon_home.join("learning"), "learning_event", 8),
        proposals: proposal_rows(&paths.archon_home, 8),
        trust_deltas: trust_rows(&paths.archon_home, 8),
        session_count: count_dirs(&paths.session_activity_root),
        recent_sessions,
        reasoning_store_present: paths
            .reasoning_quality_root
            .join("reasoning-quality.db")
            .exists(),
    }
}

fn learning_signals(paths: &WebRuntimePaths) -> Vec<LearningSignalItem> {
    vec![
        signal(
            "session activity",
            "sessions",
            paths.session_activity_root.clone(),
        ),
        signal("memory database", "memory", paths.memory_db.clone()),
        signal(
            "reasoning quality",
            "reasoning",
            paths.reasoning_quality_root.clone(),
        ),
        signal(
            "self trust",
            "calibration",
            paths
                .archon_home
                .join("self-calibration/trust/self-trust.json"),
        ),
        signal(
            "learning events",
            "learning",
            paths.archon_home.join("learning"),
        ),
        signal(
            "behaviour proposals",
            "proposal",
            paths.archon_home.join("behaviour"),
        ),
        signal(
            "behavior proposals",
            "proposal",
            paths.archon_home.join("behavior"),
        ),
    ]
}

fn signal(label: &str, kind: &str, path: PathBuf) -> LearningSignalItem {
    LearningSignalItem {
        label: label.into(),
        kind: kind.into(),
        status: if path.exists() { "present" } else { "missing" }.into(),
        count: count_entries(&path),
        path: display_path(&path),
    }
}

fn memory_rows(memory_db: &Path, limit: usize) -> Vec<LearningRowPreview> {
    if !memory_db.exists() {
        return Vec::new();
    }
    let path_str = memory_db.to_string_lossy().to_string();
    let Ok(db) = cozo::DbInstance::new("sqlite", &path_str, "") else {
        return Vec::new();
    };
    let query = format!(
        "?[id, content, title, memory_type, importance] := \
         *memories{{id, content, title, memory_type, importance}} :limit {limit}"
    );
    db.run_script(&query, Default::default(), ScriptMutability::Immutable)
        .ok()
        .into_iter()
        .flat_map(|rows| rows.rows)
        .map(|row| LearningRowPreview {
            label: cozo_text(row.get(2))
                .unwrap_or_else(|| cozo_text(row.first()).unwrap_or_default()),
            kind: cozo_text(row.get(3)).unwrap_or_else(|| "memory".into()),
            status: "recorded".into(),
            detail: cozo_text(row.get(1))
                .unwrap_or_default()
                .chars()
                .take(180)
                .collect(),
            path: display_path(memory_db),
        })
        .collect()
}

fn proposal_rows(home: &Path, limit: usize) -> Vec<LearningRowPreview> {
    [home.join("behaviour"), home.join("behavior")]
        .iter()
        .flat_map(|root| recent_files(root, "proposal", limit))
        .take(limit)
        .collect()
}

fn trust_rows(home: &Path, limit: usize) -> Vec<LearningRowPreview> {
    let trust = home.join("self-calibration/trust/self-trust.json");
    let Ok(text) = fs::read_to_string(&trust) else {
        return Vec::new();
    };
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .into_iter()
        .flat_map(|object| object.into_iter())
        .take(limit)
        .map(|(domain, value)| LearningRowPreview {
            label: domain,
            kind: "trust".into(),
            status: "recorded".into(),
            detail: compact_json(&value),
            path: display_path(&trust),
        })
        .collect()
}

fn ledger_rows(root: &Path, kind: &str, limit: usize) -> Vec<LearningRowPreview> {
    recent_files(root, kind, limit)
        .into_iter()
        .map(|mut row| {
            row.detail = latest_line(Path::new(&row.path)).unwrap_or(row.detail);
            row
        })
        .collect()
}

fn recent_files(root: &Path, kind: &str, limit: usize) -> Vec<LearningRowPreview> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut rows: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            if metadata.is_dir() {
                return Some(nested_file_preview(&path, kind, limit));
            }
            Some(vec![file_preview(&path, &metadata, kind)])
        })
        .flatten()
        .collect();
    rows.sort_by(|a, b| b.status.cmp(&a.status));
    rows.into_iter().take(limit).collect()
}

fn nested_file_preview(root: &Path, kind: &str, limit: usize) -> Vec<LearningRowPreview> {
    fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata
                .is_file()
                .then(|| file_preview(&entry.path(), &metadata, kind))
        })
        .take(limit)
        .collect()
}

fn file_preview(path: &Path, metadata: &fs::Metadata, kind: &str) -> LearningRowPreview {
    LearningRowPreview {
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
        path: display_path(path),
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

fn compact_json(value: &serde_json::Value) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "unreadable trust row".into())
        .chars()
        .take(180)
        .collect()
}

fn cozo_text(value: Option<&cozo::DataValue>) -> Option<String> {
    value?.get_str().map(str::to_string)
}

fn settings_summary() -> SettingsSummary {
    SettingsSummary {
        theme_modes: vec!["dark".into(), "light".into()],
        density_modes: vec!["comfortable".into(), "compact".into()],
        policy_editing_enabled: false,
        direct_filesystem_open_enabled: false,
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
            if !metadata.is_dir() {
                return None;
            }
            let modified = metadata.modified().ok()?;
            Some((modified, entry.file_name().to_string_lossy().to_string()))
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
    let Ok(entries) = fs::read_dir(path) else {
        return (0, 0);
    };
    entries.flatten().fold((0, 0), |(files, bytes), entry| {
        let (child_files, child_bytes) = dir_stats(&entry.path(), depth + 1);
        (files + child_files, bytes + child_bytes)
    })
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(PathProbe::decl(&cfg)),
        exported(LearningSummary::decl(&cfg)),
        exported(LearningSignalItem::decl(&cfg)),
        exported(LearningRowPreview::decl(&cfg)),
        exported(SettingsSummary::decl(&cfg)),
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
    fn settings_are_read_only_by_default() {
        let settings = settings_summary();
        assert!(!settings.policy_editing_enabled);
        assert!(!settings.direct_filesystem_open_enabled);
    }
}
