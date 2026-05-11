//! Retention and rotation helpers for world-model raw ledgers.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Result;
use chrono::Utc;

use super::jsonl;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub jsonl_rotate_bytes: u64,
    pub raw_retention_days: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            jsonl_rotate_bytes: 500 * 1024 * 1024,
            raw_retention_days: 90,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionReport {
    pub rotated_to: Option<PathBuf>,
    pub removed_files: usize,
}

pub fn apply_retention(root: &Path, policy: RetentionPolicy) -> Result<RetentionReport> {
    let rotated_to = rotate_current_ledger(root, policy.jsonl_rotate_bytes)?;
    let removed_files = remove_expired_ledgers(root, policy.raw_retention_days)?;
    Ok(RetentionReport {
        rotated_to,
        removed_files,
    })
}

fn rotate_current_ledger(root: &Path, max_bytes: u64) -> Result<Option<PathBuf>> {
    let path = jsonl::ledger_path(root);
    if !path.exists() || path.metadata()?.len() <= max_bytes {
        return Ok(None);
    }

    let rotated = path.with_file_name(format!(
        "world-trace-rows-{}.jsonl",
        Utc::now().format("%Y%m%d%H%M%S")
    ));
    fs::rename(path, &rotated)?;
    Ok(Some(rotated))
}

fn remove_expired_ledgers(root: &Path, retention_days: u64) -> Result<usize> {
    let ledger_dir = root.join("ledgers");
    if !ledger_dir.exists() {
        return Ok(0);
    }

    let max_age = Duration::from_secs(retention_days.saturating_mul(86_400));
    let mut removed = 0usize;
    for entry in fs::read_dir(ledger_dir)? {
        let path = entry?.path();
        if !is_rotated_ledger(&path) || !is_older_than(&path, max_age)? {
            continue;
        }
        fs::remove_file(path)?;
        removed += 1;
    }
    Ok(removed)
}

fn is_rotated_ledger(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with("world-trace-rows-") && name.ends_with(".jsonl"))
        .unwrap_or(false)
}

fn is_older_than(path: &Path, max_age: Duration) -> Result<bool> {
    let modified = path.metadata()?.modified().unwrap_or(SystemTime::now());
    Ok(modified.elapsed().unwrap_or_default() > max_age)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_current_ledger_when_it_exceeds_policy() {
        let temp = tempfile::tempdir().unwrap();
        let ledger = jsonl::ledger_path(temp.path());
        fs::create_dir_all(ledger.parent().unwrap()).unwrap();
        fs::write(&ledger, "this is bigger than ten bytes").unwrap();

        let report = apply_retention(
            temp.path(),
            RetentionPolicy {
                jsonl_rotate_bytes: 10,
                raw_retention_days: 90,
            },
        )
        .unwrap();

        assert!(report.rotated_to.unwrap().exists());
        assert!(!ledger.exists());
    }
}
