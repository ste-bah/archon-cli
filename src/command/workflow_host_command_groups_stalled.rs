use super::{HostCommandGroupRecord, UNKNOWN_SUFFIX, left_groups, unknown_path};
use std::path::Path;

/// The kept records of stalled teardowns that still run (or whose survivors
/// are unknown): while any exists, the run must pause and no operational
/// retry may start (Issue 270 round 3).
pub(crate) fn stalled_running(run_dir: &Path) -> anyhow::Result<Vec<HostCommandGroupRecord>> {
    let (running, _) = left_groups(run_dir)?;
    Ok(running
        .into_iter()
        .filter(|record| record.stalled || record.survivors_unknown)
        .collect())
}

/// What a stalled record adds to a refusal: who may still run.
pub(crate) fn stall_note(record: &HostCommandGroupRecord) -> String {
    if record.survivors_unknown {
        " (its teardown stalled and its survivors are unknown: verify that none of its processes runs)".to_string()
    } else if record.stalled {
        format!(
            " (its teardown stalled; survivors: {})",
            record
                .survivors
                .iter()
                .map(|(pid, _)| pid.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        String::new()
    }
}

pub(in super::super) fn refusal_files(path: &Path) -> String {
    let mut files = vec![path.to_path_buf()];
    if path.extension().is_some_and(|ext| ext == "pending") {
        let original = path.with_extension("json");
        for record in [original.clone(), unknown_path(&original)] {
            if record.exists() {
                files.push(record);
            }
        }
    } else {
        let pending = if let Some(stem) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(UNKNOWN_SUFFIX))
        {
            path.with_file_name(format!("{stem}.pending"))
        } else {
            path.with_extension("pending")
        };
        if pending.exists() {
            files.push(pending);
        }
    }
    files
        .iter()
        .map(|file| file.display().to_string())
        .collect::<Vec<_>>()
        .join(" and ")
}
