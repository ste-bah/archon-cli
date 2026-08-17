use anyhow::Result;
use archon_completion::CompletionEvidence;
use archon_session::plan::{PlanApprovalAuthority, PlanStore};

use crate::tool::AuthoritativeBashExecution;

/// Persist test evidence from the Bash tool's opaque execution metadata.
///
/// This function lives in the same crate that owns the only constructor for
/// `AuthoritativeBashExecution`; callers cannot substitute raw command/output
/// strings or implement a lookalike interface.
pub fn record_authoritative_test_execution(
    store: &PlanStore,
    authority: &PlanApprovalAuthority,
    execution: &AuthoritativeBashExecution,
) -> Result<Option<CompletionEvidence>> {
    if !is_test_command(execution.command()) {
        return Ok(None);
    }
    if execution.tool_use_id().is_empty() {
        anyhow::bail!("authoritative Bash execution is missing its tool-use identity");
    }
    store
        .record_authoritative_test_execution(
            authority,
            execution.session_id(),
            execution.tool_use_id(),
            execution.attempt(),
            execution.command(),
            execution.output(),
            execution.exit_code(),
        )
        .map(Some)
        .map_err(Into::into)
}

fn is_test_command(command: &str) -> bool {
    let command = command.trim_start();
    if command.contains([';', '|', '&', '\n', '#']) {
        return false;
    }
    let mut words = command.split_ascii_whitespace();
    matches!(
        (words.next(), words.next()),
        (Some("cargo"), Some("test"))
            | (Some("npm"), Some("test"))
            | (Some("pnpm"), Some("test"))
            | (Some("yarn"), Some("test"))
            | (Some("pytest"), _)
    )
}
