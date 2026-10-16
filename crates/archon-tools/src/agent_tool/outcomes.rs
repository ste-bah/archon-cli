//! Shared outcome handling for the foreground and detached agent paths.
use crate::background_agents::{BACKGROUND_AGENTS, BackgroundAgentHandle, RegistryError};
use crate::board::DelegatedOutcome;
use crate::subagent_executor::SubagentOutcome;

/// A runner may register before `AgentTool::execute`; that duplicate is the
/// same handle and is already registered.
pub(super) fn register_once(handle: BackgroundAgentHandle) -> Result<(), RegistryError> {
    match BACKGROUND_AGENTS.register(handle) {
        Ok(()) | Err(RegistryError::Duplicate(_)) => Ok(()),
        Err(error) => Err(error),
    }
}

/// `AutoBackgrounded` is still running, so its board item remains claimed
/// until the completion channel delivers its terminal outcome.
pub(super) fn board_outcome(outcome: &SubagentOutcome) -> Option<DelegatedOutcome> {
    match outcome {
        SubagentOutcome::Completed(_) => Some(DelegatedOutcome::Completed),
        SubagentOutcome::Failed(_) | SubagentOutcome::FailedWithProvider { .. } => {
            Some(DelegatedOutcome::Failed)
        }
        SubagentOutcome::Cancelled => Some(DelegatedOutcome::Stopped),
        SubagentOutcome::AutoBackgrounded => None,
    }
}
