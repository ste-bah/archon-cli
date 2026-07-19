//! Agent-output usability checks consumed by the write coordinator's
//! patch-manifest validation.

use crate::context;
use crate::error::{WorkflowError, WorkflowResult};

/// Reject a stage output body that self-reports blocked, failed, or
/// unverifiable status before it can be accepted as a usable artifact.
pub(crate) fn ensure_output_usable(body: &str) -> WorkflowResult<()> {
    if let Some(reason) = context::output_reports_blocked(body) {
        return Err(WorkflowError::StageFailed(reason));
    }
    if let Some(reason) = context::output_reports_failed_verification(body) {
        return Err(WorkflowError::StageFailed(reason));
    }
    Ok(())
}
