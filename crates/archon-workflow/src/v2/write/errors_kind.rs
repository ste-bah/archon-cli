//! How a write branch's error text is classified. Split from `errors.rs`
//! for the 500-line ceiling.

use super::super::*;
use super::EMPTY_REPLY_MARKER;

pub(in crate::v2::write) fn write_branch_error_kind(error: &str) -> BranchFailureKind {
    if crate::error::is_host_operational_text(error) {
        return BranchFailureKind::Execution;
    }
    let lower = root_write_branch_error(error).to_ascii_lowercase();
    if lower.contains("changed files outside declared ownership")
        || lower.contains("implementation agent changed files outside declared target_files")
        || lower.contains("changed files outside declared target_files")
        || lower.contains("changed undeclared path")
        || lower.contains("patch writes undeclared path")
        || lower.contains("declares no target ownership")
        || (lower.contains("write target") && lower.contains("is unsafe"))
        || lower.contains("read-only")
        || lower.contains("patch apply")
    {
        return BranchFailureKind::Safety;
    }
    if lower.contains(EMPTY_REPLY_MARKER)
        || lower.contains("agent transport failed")
        || lower.contains("tool execution failed")
        || lower.contains("process failed")
        || lower.contains("timed out")
        || lower.contains("rate limit")
        || lower.contains("cancelled")
    {
        return BranchFailureKind::Execution;
    }
    BranchFailureKind::Contract
}

fn root_write_branch_error(error: &str) -> &str {
    let marker = "schema repair failed after bounded retries: root=";
    let Some(root_and_last) = error.strip_prefix(marker) else {
        return error;
    };
    root_and_last
        .split_once("; last=")
        .map(|(root, _)| root)
        .unwrap_or(root_and_last)
}
