// Durable records of what a write wave applied and verified.
//
// Split out of `patch_apply.rs` to keep that file under the 500-line cap the
// repository guard enforces. These are the only writers of the
// `write-coordination/stages/<stage>/` tree, so keeping them together makes the
// on-disk layout readable in one place.

use std::path::{Path, PathBuf};

use super::{ApplyError, ApplyRecord, VerifyResult, WaveId};

pub(super) fn persist_record(
    run_root: &Path,
    stage_id: &str,
    wave_id: WaveId,
    rec: &ApplyRecord,
) -> Result<(), ApplyError> {
    let dir = stage_dir(run_root, stage_id).join("apply");
    let json = serde_json::to_vec_pretty(rec).map_err(|e| ApplyError::PersistFailed {
        source: std::io::Error::other(e),
    })?;
    write_atomic(&dir.join(format!("{wave_id}.json")), &json)
}

pub(super) fn persist_verify(
    run_root: &Path,
    stage_id: &str,
    wave_id: WaveId,
    result: &VerifyResult,
) -> Result<(), ApplyError> {
    let dir = stage_dir(run_root, stage_id).join("tests");
    let json = serde_json::to_vec_pretty(result).map_err(|e| ApplyError::PersistFailed {
        source: std::io::Error::other(e),
    })?;
    write_atomic(&dir.join(format!("{wave_id}.json")), &json)
}

fn stage_dir(run_root: &Path, stage_id: &str) -> PathBuf {
    run_root
        .join("write-coordination")
        .join("stages")
        .join(stage_id)
}

/// Write through a temporary file and rename, so a reader never observes a
/// partially written record.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ApplyError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ApplyError::PersistFailed { source })?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|source| ApplyError::PersistFailed { source })?;
    std::fs::rename(&tmp, path).map_err(|source| ApplyError::PersistFailed { source })?;
    Ok(())
}

pub(super) fn persist_io(
    e: crate::write_coordinator::patch_manifest::PatchError,
) -> std::io::Error {
    std::io::Error::other(e.to_string())
}
