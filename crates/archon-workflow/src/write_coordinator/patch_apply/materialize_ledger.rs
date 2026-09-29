//! The run's append-only record of every copy a landing placed (Issue-113).
//!
//! A manifest is rewritten whenever its item is captured again: a later
//! replay of the same item as a no-op persists a manifest with no receipts,
//! and a check that read receipts from the manifest then saw "nothing was
//! ever copied" -- delete the copy and the landing was credited anyway. So
//! the manifest's `materialized` map is informational; THIS file is the
//! authority. A line is appended once per copy, when its landing is decided,
//! and never rewritten. A reader that cannot parse every line fails closed.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::write_coordinator::patch_manifest::MaterializedDeliverable;

/// One copy a landing placed: which landing, which declared path, and its
/// receipt (destination, pre/post state, run-wide sequence).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RunMaterialization {
    pub(crate) stage_id: String,
    pub(crate) item_id: String,
    pub(crate) path: String,
    pub(crate) receipt: MaterializedDeliverable,
    /// When the copy was placed, in nanoseconds since the epoch; 0 on a line
    /// written before the time was recorded (Batch L).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(crate) at: i64,
    /// Batch L: the host put the destination back because a verdict of the
    /// landing's unit refused it; `receipt` runs from the copy's state to the
    /// restored one, in the run-wide order.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) reverted: bool,
}

fn is_zero(at: &i64) -> bool {
    *at == 0
}

fn ledger_path(run_root: &Path) -> PathBuf {
    run_root
        .join("write-coordination")
        .join("materializations.jsonl")
}

/// Every copy this run's landings placed, in append order. No ledger is an
/// empty answer; a line that does not parse is an error, never skipped.
pub(crate) fn run_materializations(run_root: &Path) -> Result<Vec<RunMaterialization>, String> {
    let path = ledger_path(run_root);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line)
                .map_err(|error| format!("{} line {}: {error}", path.display(), index + 1))
        })
        .collect()
}

/// Append the host's revert of a refused copy (Batch L), flushed to disk.
pub(crate) fn append_revert(run_root: &Path, line: &RunMaterialization) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(line).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    let path = ledger_path(run_root);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(&bytes)?;
    file.sync_all()
}

/// Append `placed` for `(stage_id, item_id)`, flushed to disk before return.
pub(crate) fn append(
    run_root: &Path,
    stage_id: &str,
    item_id: &str,
    placed: &[(String, MaterializedDeliverable)],
) -> std::io::Result<()> {
    if placed.is_empty() {
        return Ok(());
    }
    let path = ledger_path(run_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut bytes = Vec::new();
    for (rel, receipt) in placed {
        let line = RunMaterialization {
            stage_id: stage_id.to_string(),
            item_id: item_id.to_string(),
            path: rel.clone(),
            receipt: receipt.clone(),
            at: chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            reverted: false,
        };
        serde_json::to_writer(&mut bytes, &line).map_err(std::io::Error::other)?;
        bytes.push(b'\n');
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(&bytes)?;
    file.sync_all()
}
