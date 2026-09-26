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
