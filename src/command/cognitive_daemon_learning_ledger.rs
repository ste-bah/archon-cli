use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LearningDaemonEvent {
    pub created_at: DateTime<Utc>,
    pub job: String,
    pub status: String,
    pub summary: String,
    pub trained: bool,
    pub run_id: Option<String>,
    pub total_memories: Option<u64>,
    pub total_corrections: Option<u64>,
}

impl LearningDaemonEvent {
    pub(crate) fn new(
        job: impl Into<String>,
        status: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            created_at: Utc::now(),
            job: job.into(),
            status: status.into(),
            summary: summary.into(),
            trained: false,
            run_id: None,
            total_memories: None,
            total_corrections: None,
        }
    }
}

pub(crate) fn append(root: &Path, event: &LearningDaemonEvent) -> Result<()> {
    std::fs::create_dir_all(root)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path(root))?;
    let mut line = serde_json::to_vec(event)?;
    line.push(b'\n');
    file.write_all(&line)?;
    Ok(())
}

pub(crate) fn latest(root: &Path, limit: usize) -> Vec<LearningDaemonEvent> {
    let Ok(raw) = std::fs::read_to_string(path(root)) else {
        return Vec::new();
    };
    raw.lines()
        .rev()
        .filter_map(|line| serde_json::from_str(line).ok())
        .take(limit)
        .collect()
}

pub(crate) fn latest_for_job(root: &Path, job: &str) -> Option<LearningDaemonEvent> {
    latest(root, 256).into_iter().find(|event| event.job == job)
}

#[cfg(test)]
pub(crate) fn latest_training_counts(root: &Path, job: &str) -> Option<(u64, u64)> {
    latest(root, 512)
        .into_iter()
        .find(|event| {
            event.job == job
                && event.trained
                && event.total_memories.is_some()
                && event.total_corrections.is_some()
        })
        .map(|event| {
            (
                event.total_memories.unwrap_or_default(),
                event.total_corrections.unwrap_or_default(),
            )
        })
}

pub(crate) fn latest_count_checkpoint(root: &Path, job: &str) -> Option<(u64, u64)> {
    latest(root, 512)
        .into_iter()
        .find(|event| {
            event.job == job && event.total_memories.is_some() && event.total_corrections.is_some()
        })
        .map(|event| {
            (
                event.total_memories.unwrap_or_default(),
                event.total_corrections.unwrap_or_default(),
            )
        })
}

pub(crate) fn render_summary(root: &Path) -> String {
    let events = latest(root, 5);
    if events.is_empty() {
        return "Learning daemon jobs: no recorded decisions".into();
    }
    let rows = events
        .iter()
        .map(|event| {
            format!(
                "{} {} {} — {}",
                event.created_at.to_rfc3339(),
                event.job,
                event.status,
                event.summary
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("Learning daemon jobs:\n{rows}")
}

pub(crate) fn path(root: &Path) -> PathBuf {
    root.join("learning-daemon-events.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_for_job_reads_newest_matching_event() {
        let temp = tempfile::tempdir().unwrap();
        append(
            temp.path(),
            &LearningDaemonEvent::new("world", "skipped", "old"),
        )
        .unwrap();
        append(
            temp.path(),
            &LearningDaemonEvent::new("gnn", "ok", "trained"),
        )
        .unwrap();

        let event = latest_for_job(temp.path(), "gnn").unwrap();

        assert_eq!(event.summary, "trained");
    }

    #[test]
    fn latest_training_counts_ignores_skip_events() {
        let temp = tempfile::tempdir().unwrap();
        let mut trained = LearningDaemonEvent::new("gnn", "trained", "ok");
        trained.trained = true;
        trained.total_memories = Some(10);
        trained.total_corrections = Some(2);
        append(temp.path(), &trained).unwrap();
        append(
            temp.path(),
            &LearningDaemonEvent::new("gnn", "skipped", "later"),
        )
        .unwrap();

        assert_eq!(latest_training_counts(temp.path(), "gnn"), Some((10, 2)));
    }
}
