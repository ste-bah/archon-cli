use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use super::{
    inspect::PathProbe, world::WorldModelArtifact, world::WorldModelRowPreview,
    world::WorldModelSignal,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct JepaInspectionSummary {
    pub root: PathProbe,
    pub candidate_count: u64,
    pub eval_count: u64,
    pub training_run_count: u64,
    pub comparison_count: u64,
    pub artifacts: Vec<WorldModelArtifact>,
    pub signals: Vec<WorldModelSignal>,
    pub candidates: Vec<WorldModelRowPreview>,
    pub evals: Vec<WorldModelRowPreview>,
    pub training_runs: Vec<WorldModelRowPreview>,
    pub comparisons: Vec<WorldModelRowPreview>,
}

pub(crate) fn jepa_summary(world_root: &Path) -> JepaInspectionSummary {
    let root = world_root.join("jepa");
    let candidate_dir = root.join("candidates");
    let eval_dir = root.join("evals");
    let training_dir = root.join("training-runs");
    let comparison_dir = root.join("representation-comparisons");
    JepaInspectionSummary {
        root: probe("JEPA", root.clone()),
        candidate_count: count_entries(&candidate_dir),
        eval_count: count_entries(&eval_dir),
        training_run_count: count_entries(&training_dir),
        comparison_count: count_entries(&comparison_dir),
        artifacts: vec![
            artifact("JEPA root", "jepa", root),
            artifact("candidate registry", "candidate", candidate_dir.clone()),
            artifact("eval reports", "eval", eval_dir.clone()),
            artifact("training runs", "training", training_dir.clone()),
            artifact(
                "representation comparisons",
                "comparison",
                comparison_dir.clone(),
            ),
        ],
        signals: jepa_signals(&candidate_dir, &eval_dir, &training_dir),
        candidates: row_previews(&candidate_dir, "candidate", 8),
        evals: row_previews(&eval_dir, "eval", 8),
        training_runs: row_previews(&training_dir, "training", 8),
        comparisons: row_previews(&comparison_dir, "comparison", 8),
    }
}

fn jepa_signals(
    candidate_dir: &Path,
    eval_dir: &Path,
    training_dir: &Path,
) -> Vec<WorldModelSignal> {
    let (eval_status, eval_detail) = latest_eval_status(eval_dir);
    vec![
        signal(
            "Candidate registry",
            exists_status(candidate_dir),
            format!("{} candidate files", count_entries(candidate_dir)),
        ),
        signal("Latest eval gate", eval_status, eval_detail),
        signal(
            "Latest training run",
            exists_status(training_dir),
            latest_training_detail(training_dir),
        ),
    ]
}

fn latest_eval_status(eval_dir: &Path) -> (String, String) {
    let Some(path) = latest_file(eval_dir) else {
        return ("missing".into(), "no JEPA eval report found".into());
    };
    let Ok(value) = fs::read_to_string(&path)
        .and_then(|text| serde_json::from_str::<Value>(&text).map_err(std::io::Error::other))
    else {
        return ("stale".into(), path.to_string_lossy().to_string());
    };
    let passed = value
        .pointer("/gates/passed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let detail = value
        .get("candidate_id")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("eval")
        });
    (
        if passed { "passed" } else { "failed" }.into(),
        detail.to_string(),
    )
}

fn latest_training_detail(training_dir: &Path) -> String {
    latest_file(training_dir)
        .and_then(|path| latest_line(&path))
        .unwrap_or_else(|| "no JEPA training run found".into())
}

fn row_previews(root: &Path, kind: &str, limit: usize) -> Vec<WorldModelRowPreview> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut rows: Vec<_> = entries
        .flatten()
        .filter_map(|entry| file_row(&entry.path(), kind))
        .collect();
    rows.sort_by(|a, b| b.status.cmp(&a.status));
    rows.into_iter().take(limit).collect()
}

fn file_row(path: &Path, kind: &str) -> Option<WorldModelRowPreview> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.is_dir() {
        return None;
    }
    Some(WorldModelRowPreview {
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
    })
}

fn latest_line(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()?
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.chars().take(180).collect())
}

fn latest_file(root: &Path) -> Option<PathBuf> {
    fs::read_dir(root)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata
                .is_file()
                .then(|| (entry.path(), metadata.modified().ok()))
        })
        .max_by_key(|(_, modified)| *modified)
        .map(|(path, _)| path)
}

fn signal(label: &str, status: impl Into<String>, detail: impl Into<String>) -> WorldModelSignal {
    WorldModelSignal {
        label: label.into(),
        status: status.into(),
        detail: detail.into(),
    }
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

fn exists_status(path: &Path) -> String {
    if path.exists() { "present" } else { "missing" }.into()
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
