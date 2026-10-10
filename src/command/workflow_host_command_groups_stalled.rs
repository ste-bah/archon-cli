use super::{HostCommandGroupRecord, left_groups};
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
