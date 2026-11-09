//! Claims are discharged only by the host's stored tip/base comparison.
//! Summaries and typed owner lists can describe a failure but cannot prove its identity.

#[cfg(test)]
use serde_json::Value;

use super::baseline_rule::BaselineStamp;
use crate::v2::{WorkflowV2CommandKind, WorkflowV2CommandStatus};

/// Failed declared commands claimed pre_existing, split by proof and stale declarations.
#[derive(Debug, Default)]
pub(super) struct PreExistingClaims {
    /// Nothing ties the failure to another task's test: the claim is refused.
    pub(super) unproven: Vec<String>,
    /// The report says zero tests matched; recorded as a stale declaration.
    pub(super) zero_match: Vec<String>,
}

/// Report stale zero-match declarations and refuse every claim lacking host proof.
pub(super) fn pre_existing_claims(
    result: &crate::WorkflowV2Result,
    stamp: &BaselineStamp,
    proven: &[String],
) -> PreExistingClaims {
    let mut claims = PreExistingClaims::default();
    for command in result
        .commands_run
        .iter()
        .filter(|command| command.kind == WorkflowV2CommandKind::Test)
        .filter(|command| command.status == WorkflowV2CommandStatus::Failed && command.pre_existing)
        .filter(|command| !super::baseline_demo::is_baseline_demonstration(result, command))
        .filter(|command| {
            crate::context::command_matches_declared_focused_test(
                &command.command,
                &stamp.declared_commands,
            )
        })
    {
        if crate::context::command_output_reports_zero_matched_tests(
            &command.command,
            &command.output_summary,
        ) && crate::v2::write::test_baseline_parse::failing_tests(&command.output_summary)
            .is_empty()
            && super::is_evidenced_pre_existing_failure(command)
        {
            claims.zero_match.push(command.command.clone());
        }
        if !proven.iter().any(|done| done == command.command.trim()) {
            claims.unproven.push(command.command.clone());
        }
    }
    claims
}

/// The typed failing names the verifier declared, read strictly: `Some` only
/// for an array of non-empty strings. Any other shape is malformed, and a
/// malformed declaration proves nothing.
#[cfg(test)]
pub(super) fn declared_failed_names(data: &Value) -> Option<Vec<String>> {
    let entries = data
        .get("matched_test_check_names")?
        .get("failed")?
        .as_array()?;
    let mut names = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = entry.as_str()?.trim();
        if name.is_empty() {
            return None;
        }
        names.push(name.to_string());
    }
    Some(names)
}

/// Whether the host's routing table carries every name in `names` and routes
/// each of them to a task other than the one under verification.
#[cfg(test)]
pub(super) fn routed_to_other_tasks(names: &[String], stamp: &BaselineStamp) -> bool {
    if names.is_empty() || stamp.tasks.is_empty() {
        return false;
    }
    names.iter().all(|name| {
        let mut routed = stamp
            .other_owner
            .iter()
            .filter(|entry| &entry.test_id == name)
            .peekable();
        routed.peek().is_some() && routed.all(|entry| !stamp.tasks.contains(&entry.owner_task))
    })
}

#[cfg(test)]
#[path = "baseline_pre_existing_tests.rs"]
mod tests;
