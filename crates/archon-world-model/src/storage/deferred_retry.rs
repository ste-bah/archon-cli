use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::WorldModelStore;

const SHADOW_EVIDENCE_RETRY_FILE: &str = "deferred-shadow-evidence-retry.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeferredShadowEvidenceRetry {
    pub retry_id: String,
    pub reason: String,
    pub fail_open_count: u64,
    pub retry_attempts: u64,
    pub first_recorded_at: DateTime<Utc>,
    pub last_recorded_at: DateTime<Utc>,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ShadowEvidenceRetryOutcome {
    NoPending,
    Resolved {
        retry_id: String,
        attempts: u64,
        rows_loaded: usize,
    },
    StillPending {
        retry_id: String,
        attempts: u64,
        last_error: String,
    },
}

pub fn record_shadow_evidence_retry(
    root: impl AsRef<Path>,
    reason: impl Into<String>,
) -> Result<DeferredShadowEvidenceRetry> {
    let root = root.as_ref();
    let now = Utc::now();
    let mut marker =
        read_shadow_evidence_retry(root)?.unwrap_or_else(|| DeferredShadowEvidenceRetry {
            retry_id: Uuid::new_v4().to_string(),
            reason: String::new(),
            fail_open_count: 0,
            retry_attempts: 0,
            first_recorded_at: now,
            last_recorded_at: now,
            last_attempt_at: None,
            last_error: None,
        });
    let reason = reason.into();
    marker.reason = reason.clone();
    marker.last_error = Some(reason);
    marker.fail_open_count = marker.fail_open_count.saturating_add(1);
    marker.last_recorded_at = now;
    write_shadow_evidence_retry(root, &marker)?;
    Ok(marker)
}

pub fn read_shadow_evidence_retry(
    root: impl AsRef<Path>,
) -> Result<Option<DeferredShadowEvidenceRetry>> {
    let path = retry_path(root.as_ref());
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read(&path)
        .with_context(|| format!("read deferred world-model retry marker {}", path.display()))?;
    serde_json::from_slice(&raw)
        .with_context(|| format!("parse deferred world-model retry marker {}", path.display()))
        .map(Some)
}

pub fn clear_shadow_evidence_retry(root: impl AsRef<Path>) -> Result<bool> {
    let path = retry_path(root.as_ref());
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| {
            format!("clear deferred world-model retry marker {}", path.display())
        })?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn process_shadow_evidence_retry(root: impl AsRef<Path>) -> Result<ShadowEvidenceRetryOutcome> {
    let root = root.as_ref();
    let Some(mut marker) = read_shadow_evidence_retry(root)? else {
        return Ok(ShadowEvidenceRetryOutcome::NoPending);
    };
    marker.retry_attempts = marker.retry_attempts.saturating_add(1);
    marker.last_attempt_at = Some(Utc::now());

    match WorldModelStore::open(root).and_then(|store| store.load_rows()) {
        Ok(rows) => {
            clear_shadow_evidence_retry(root)?;
            Ok(ShadowEvidenceRetryOutcome::Resolved {
                retry_id: marker.retry_id,
                attempts: marker.retry_attempts,
                rows_loaded: rows.len(),
            })
        }
        Err(error) => {
            marker.last_error = Some(error.to_string());
            write_shadow_evidence_retry(root, &marker)?;
            Ok(ShadowEvidenceRetryOutcome::StillPending {
                retry_id: marker.retry_id,
                attempts: marker.retry_attempts,
                last_error: error.to_string(),
            })
        }
    }
}

fn write_shadow_evidence_retry(root: &Path, marker: &DeferredShadowEvidenceRetry) -> Result<()> {
    std::fs::create_dir_all(root)
        .with_context(|| format!("create world-model retry marker root {}", root.display()))?;
    let path = retry_path(root);
    let tmp_path = tmp_retry_path(root);
    let body = serde_json::to_vec_pretty(marker)?;
    std::fs::write(&tmp_path, body).with_context(|| {
        format!(
            "write deferred world-model retry marker {}",
            tmp_path.display()
        )
    })?;
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| {
            format!(
                "replace deferred world-model retry marker {}",
                path.display()
            )
        })?;
    }
    std::fs::rename(&tmp_path, &path).with_context(|| {
        format!(
            "commit deferred world-model retry marker {}",
            path.display()
        )
    })?;
    Ok(())
}

fn retry_path(root: &Path) -> PathBuf {
    root.join(SHADOW_EVIDENCE_RETRY_FILE)
}

fn tmp_retry_path(root: &Path) -> PathBuf {
    root.join(format!("{SHADOW_EVIDENCE_RETRY_FILE}.tmp"))
}

#[cfg(test)]
mod tests {
    use crate::schema::{WorldActionKind, WorldTraceRow};

    use super::*;

    #[test]
    fn record_shadow_retry_upserts_marker() {
        let temp = tempfile::tempdir().unwrap();

        let first = record_shadow_evidence_retry(temp.path(), "database is locked").unwrap();
        let second = record_shadow_evidence_retry(temp.path(), "still locked").unwrap();

        assert_eq!(first.retry_id, second.retry_id);
        assert_eq!(second.fail_open_count, 2);
        assert_eq!(second.last_error.as_deref(), Some("still locked"));
        assert!(read_shadow_evidence_retry(temp.path()).unwrap().is_some());
    }

    #[test]
    fn process_shadow_retry_resolves_when_store_loads() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorldModelStore::open(temp.path()).unwrap();
        let row = WorldTraceRow::new("session-1", WorldActionKind::ToolCall).with_row_id("row-1");
        store.persist_rows(&[row]).unwrap();
        record_shadow_evidence_retry(temp.path(), "database is locked").unwrap();

        let outcome = process_shadow_evidence_retry(temp.path()).unwrap();

        assert!(matches!(
            outcome,
            ShadowEvidenceRetryOutcome::Resolved { rows_loaded: 1, .. }
        ));
        assert!(read_shadow_evidence_retry(temp.path()).unwrap().is_none());
    }

    #[test]
    fn process_shadow_retry_noops_without_marker() {
        let temp = tempfile::tempdir().unwrap();
        let outcome = process_shadow_evidence_retry(temp.path()).unwrap();
        assert_eq!(outcome, ShadowEvidenceRetryOutcome::NoPending);
    }
}
