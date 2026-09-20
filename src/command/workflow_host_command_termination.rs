//! Platform-specific termination; Unix retains process-group cleanup.
use archon_workflow::{WorkflowError, WorkflowResult};

use super::REAP_DEADLINE;
#[cfg(unix)]
use super::{CLEANUP_GRACE, audit_no_descendants, signal_group};

#[cfg(unix)]
pub(super) async fn terminate_and_reap(
    child: &mut tokio::process::Child,
    process_group: Option<u32>,
) -> WorkflowResult<()> {
    signal_group(process_group, libc::SIGTERM)?;
    tokio::time::sleep(CLEANUP_GRACE).await;
    signal_group(process_group, libc::SIGKILL)?;
    tokio::time::timeout(REAP_DEADLINE, child.wait())
        .await
        .map_err(|_| {
            WorkflowError::StageFailed(
                "host command process reap exceeded cleanup deadline".to_string(),
            )
        })?
        .map_err(|error| {
            WorkflowError::StageFailed(format!("host command process reap failed: {error}"))
        })?;
    Ok(())
}

#[cfg(unix)]
pub(super) async fn terminate_completed_group(process_group: Option<u32>) -> WorkflowResult<()> {
    signal_group(process_group, libc::SIGKILL)?;
    audit_no_descendants(process_group).await
}

#[cfg(not(unix))]
pub(super) async fn terminate_and_reap(
    child: &mut tokio::process::Child,
    _process_group: Option<u32>,
) -> WorkflowResult<()> {
    child.start_kill().map_err(|error| {
        WorkflowError::StageFailed(format!("host command termination failed: {error}"))
    })?;
    tokio::time::timeout(REAP_DEADLINE, child.wait())
        .await
        .map_err(|_| {
            WorkflowError::StageFailed(
                "host command process reap exceeded cleanup deadline".to_string(),
            )
        })?
        .map_err(|error| {
            WorkflowError::StageFailed(format!("host command process reap failed: {error}"))
        })?;
    Ok(())
}

#[cfg(not(unix))]
pub(super) async fn terminate_completed_group(_process_group: Option<u32>) -> WorkflowResult<()> {
    Ok(())
}
