//! Validate allowance changes before displaying and again under the run lock.
use crate::cli_args::workflow_audit::AuditAction;
use archon_workflow::{WorkflowError, WorkflowResult};
use archon_workflow::repository_audit::budget::{AuditPolicy, Limit};

fn invalid(reason: &str) -> WorkflowError { WorkflowError::PolicyDenied(format!("audit control: {reason}")) }
fn add(limit: Limit, amount: Option<u64>) -> WorkflowResult<Limit> {
    let Some(amount) = amount else { return Ok(limit); };
    if amount == 0 { return Err(invalid("extension must be positive")); }
    let Limit::Finite(current) = limit else { return Err(invalid("cannot extend an unlimited dimension")); };
    let next = current.checked_add(amount).filter(|n| *n <= i64::MAX as u64 / 1000)
        .ok_or_else(|| invalid("extension overflows supported allowance"))?;
    Ok(Limit::Finite(next))
}
fn replace(limit: Limit, value: &Option<String>) -> WorkflowResult<Limit> {
    let Some(value) = value else { return Ok(limit); };
    if value == "unlimited" { return Ok(Limit::Unlimited); }
    if !value.bytes().all(|b| b.is_ascii_digit()) { return Err(invalid("limit must be a positive integer or unlimited")); }
    let n = value.parse::<u64>().ok().filter(|n| *n > 0 && *n <= i64::MAX as u64 / 1000)
        .ok_or_else(|| invalid("invalid or overflowing limit"))?;
    Ok(Limit::Finite(n))
}
pub(super) fn updated_policy(action: &AuditAction, current: &AuditPolicy) -> WorkflowResult<AuditPolicy> {
    let mut policy = current.clone();
    let reason = match action {
        AuditAction::ExtendBudget{extra_refreshes,extra_seconds,reason,..} => {
            if extra_refreshes.is_none() && extra_seconds.is_none() { return Err(invalid("select an extension")); }
            policy.total_time_secs = add(policy.total_time_secs, *extra_seconds)?;
            policy.unexpected_change_refreshes = add(policy.unexpected_change_refreshes, *extra_refreshes)?;
            reason
        }
        AuditAction::SetBudget{attempt_timeout_secs,total_time_secs,unexpected_change_refreshes,reason,..} => {
            if attempt_timeout_secs.is_none() && total_time_secs.is_none() && unexpected_change_refreshes.is_none() {
                return Err(invalid("select at least one budget dimension"));
            }
            policy.attempt_timeout_secs = replace(policy.attempt_timeout_secs, attempt_timeout_secs)?;
            policy.total_time_secs = replace(policy.total_time_secs, total_time_secs)?;
            policy.unexpected_change_refreshes = replace(policy.unexpected_change_refreshes, unexpected_change_refreshes)?;
            reason
        }
        _ => return Err(invalid("this action requires finding-specific authorization")),
    };
    if reason.trim().is_empty() || reason.len() > 2048 || reason.chars().any(char::is_control) {
        return Err(invalid("reason must be 1..2048 bytes without control characters"));
    }
    if &policy == current { return Err(invalid("request makes no allowance change")); }
    Ok(policy)
}
