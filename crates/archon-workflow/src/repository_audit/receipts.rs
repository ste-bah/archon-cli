//! Apply receipts: the host's own record of the sealed identity an applied
//! wave moved the repository between. A later refresh that lands on a
//! receipt's `after` is the ordinary post-apply audit (possibly one a pause
//! interrupted), never an unexpected change (Issue-25).
use crate::{WorkflowError, WorkflowResult, WorkflowStore};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Run-relative directory holding `apply-<call>-<wave>.json`.
pub const RECEIPT_DIR: &str = "v2/repository-audit";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApplyReceipt {
    pub commit: String,
    pub items_applied: Vec<String>,
    /// Snapshot identity the wave was dispatched against.
    pub before: String,
    /// Snapshot identity captured right after the wave applied.
    pub after: String,
    /// Changed paths no applied manifest accounted for.
    pub unexpected_paths: Vec<String>,
    /// The write call whose manifests `items_applied` name. Empty on receipts
    /// written before it was recorded; such a receipt still proves `after`.
    #[serde(default)]
    pub call_id: String,
}

impl ApplyReceipt {
    /// Run-relative path of the receipt for one applied wave.
    pub fn relative_path(call_segment: &str, wave_id: u32) -> String {
        format!("{RECEIPT_DIR}/apply-{call_segment}-{wave_id}.json")
    }
}

/// Every apply receipt of `run_id`, in file-name order; empty when no wave
/// has applied yet.
pub fn read_apply_receipts(
    store: &WorkflowStore,
    run_id: &str,
) -> WorkflowResult<Vec<ApplyReceipt>> {
    read_receipt_dir(&store.run_dir(run_id).join(RECEIPT_DIR))
}

fn read_receipt_dir(dir: &Path) -> WorkflowResult<Vec<ApplyReceipt>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(WorkflowError::io(dir, error)),
    };
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let path = entry.map_err(|error| WorkflowError::io(dir, error))?.path();
        let is_receipt = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("apply-") && name.ends_with(".json"));
        if is_receipt {
            paths.push(path);
        }
    }
    paths.sort();
    paths
        .iter()
        .map(|path| {
            let bytes = std::fs::read(path).map_err(|error| WorkflowError::io(path, error))?;
            Ok(serde_json::from_slice(&bytes)?)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_directory_means_no_receipts() {
        let temp = tempfile::tempdir().unwrap();
        assert!(
            read_receipt_dir(&temp.path().join("absent"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn reads_receipts_written_before_call_id_was_recorded() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("apply-agents-7-0.json"),
            r#"{"after":"y","before":"x","commit":"c","items_applied":["agents-7-0"],"unexpected_paths":["p.rs"]}"#,
        )
        .unwrap();
        std::fs::write(temp.path().join("apply-agents-8-0.tmp"), "{").unwrap();
        std::fs::write(temp.path().join("state.json"), "{}").unwrap();
        let receipts = read_receipt_dir(temp.path()).unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].after, "y");
        assert_eq!(receipts[0].call_id, "");
        assert_eq!(receipts[0].unexpected_paths, vec!["p.rs".to_string()]);
    }
}
