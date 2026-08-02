use super::*;

pub(super) fn write_branch_validation_error_result(
    item_id: &str,
    input: Option<&serde_json::Value>,
    error: &str,
) -> WorkflowV2Result {
    let failure_kind = write_branch_error_kind(error);
    let (status, evidence_kind, severity) = branch_validation_failure_fields(&failure_kind);
    let canonical_task_ids = canonical_task_ids_from_write_error_input(input);
    let mut result = WorkflowV2Result {
        status,
        summary: format!(
            "write branch '{item_id}' produced invalid implementation evidence after repair"
        ),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        evidence_kind,
        "write branch validation failure was retained as typed remediation data for workflow.js",
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "invalid_write_branch_output_{}",
            sanitize_v2_path_segment(item_id)
        ),
        description: truncate_for_result(error, 500),
        severity: Some(severity.to_string()),
    });
    // A branch that needed a file outside its declared write scope is a SCOPE
    // problem, not defective work: the agent may have produced the correct fix
    // and had it discarded. Emit a separate typed gap naming the wanted path(s)
    // so the next remediation (or the authored script) can declare that scope
    // instead of re-deriving the diagnosis from scratch and failing identically.
    // Generic: paths are extracted from the runtime's own error text.
    let wanted_paths = undeclared_write_paths(error);
    if !wanted_paths.is_empty() {
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: format!(
                "scope_expansion_needed_{}",
                sanitize_v2_path_segment(item_id)
            ),
            description: truncate_for_result(
                &format!(
                    "write branch '{item_id}' required write access to path(s) outside its declared target_files: {}. \
                     The change was rejected and discarded, so this task cannot be completed until the declared write \
                     scope includes those path(s) (or the work is redirected to an in-scope file). Re-run this task with \
                     the path(s) declared in target_files.",
                    wanted_paths.join(", ")
                ),
                500,
            ),
            severity: Some(severity.to_string()),
        });
    }
    result.data = serde_json::json!({
        "branch_id": item_id,
        "item_id": item_id,
        "canonical_task_ids": canonical_task_ids,
        "branch_error_from_runtime": true,
        "failure_kind": failure_kind,
        "error": truncate_for_result(error, 2_000),
    });
    result
}

/// Extract the repository path(s) a rejected write wanted, from the runtime's own
/// ownership/scope error text. Domain-neutral: matches the quoted path the write
/// guards report, plus the unquoted `target_files: <paths>` tail form. Returns an
/// empty vec for any error that is not a scope rejection.
pub(super) fn undeclared_write_paths(error: &str) -> Vec<String> {
    let lower = error.to_ascii_lowercase();
    let is_scope_error = lower.contains("undeclared path")
        || lower.contains("outside declared target_files")
        || lower.contains("outside declared ownership");
    if !is_scope_error {
        return Vec::new();
    }
    let mut paths = Vec::new();
    // Quoted form: ...changed undeclared path 'src/foo.rs'
    let mut rest = error;
    while let Some(start) = rest.find('\'') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('\'') else { break };
        let candidate = after[..end].trim();
        if candidate.contains('/') || candidate.contains(".rs") {
            let candidate = candidate.to_string();
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
        rest = &after[end + 1..];
    }
    // Unquoted tail form: ...outside declared target_files: src/a.rs, src/b.rs
    if paths.is_empty()
        && let Some(index) = lower.find("target_files:")
    {
        for candidate in error[index + "target_files:".len()..].split(',') {
            let candidate = candidate.trim().trim_end_matches(['.', ';']).trim();
            if candidate.contains('/') && !candidate.contains(' ') {
                let candidate = candidate.to_string();
                if !paths.contains(&candidate) {
                    paths.push(candidate);
                }
            }
        }
    }
    paths.truncate(8);
    paths
}

pub(super) fn canonical_task_ids_from_write_error_input(input: Option<&serde_json::Value>) -> Vec<String> {
    let Some(input) = input else {
        return Vec::new();
    };
    let source = input.get("item").unwrap_or(input);
    canonical_task_ids_from_generated_value(source, None)
}

pub(super) fn branch_validation_failure_fields(
    failure_kind: &BranchFailureKind,
) -> (WorkflowV2Status, WorkflowV2EvidenceKind, &'static str) {
    match failure_kind {
        BranchFailureKind::Safety | BranchFailureKind::Execution => (
            WorkflowV2Status::Failed,
            WorkflowV2EvidenceKind::Blocker,
            "blocking",
        ),
        BranchFailureKind::Semantic | BranchFailureKind::Contract => (
            WorkflowV2Status::NeedsReview,
            WorkflowV2EvidenceKind::Review,
            "review",
        ),
    }
}

pub(super) fn write_branch_runtime_timeout_result(
    item_id: &str,
    input: &serde_json::Value,
    error: &str,
) -> WorkflowV2Result {
    let source = input.get("item").unwrap_or(input);
    let canonical_task_ids = canonical_task_ids_from_generated_value(source, None);
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: format!("write branch '{item_id}' timed out before returning usable output"),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "write branch timeout was retained as item-level remediation data for workflow.js",
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!("write_branch_timeout_{}", sanitize_v2_path_segment(item_id)),
        description: truncate_for_result(error, 500),
        severity: Some("review".to_string()),
    });
    result.data = serde_json::json!({
        "branch_id": item_id,
        "item_id": item_id,
        "canonical_task_ids": canonical_task_ids,
        "branch_runtime_timeout": true,
        "failure_kind": BranchFailureKind::Contract,
        "error": truncate_for_result(error, 2_000),
    });
    result
}

pub(super) fn failure_kind_from_write_result(result: &WorkflowV2Result) -> Option<BranchFailureKind> {
    result
        .data
        .get("failure_kind")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .or(match result.status {
            WorkflowV2Status::Failed
            | WorkflowV2Status::Blocked
            | WorkflowV2Status::NeedsReview => Some(BranchFailureKind::Semantic),
            WorkflowV2Status::Cancelled => Some(BranchFailureKind::Execution),
            _ => None,
        })
}

pub(super) fn is_recoverable_write_branch_timeout(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("subagent timed out") || lower.contains("timed out after")
}

pub(super) fn write_branch_error_kind(error: &str) -> BranchFailureKind {
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
    if lower.contains("agent transport failed")
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

pub(super) fn root_write_branch_error(error: &str) -> &str {
    let marker = "schema repair failed after bounded retries: root=";
    let Some(root_and_last) = error.strip_prefix(marker) else {
        return error;
    };
    root_and_last
        .split_once("; last=")
        .map(|(root, _)| root)
        .unwrap_or(root_and_last)
}

pub(super) fn is_write_branch_validation_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    !lower.contains("agent transport failed")
        && (lower.contains("schema repair failed")
            || lower.contains("agent output contains a confirmation question")
            || lower.contains("workflowv2result object")
            || lower.contains("agent result failed validation")
            || lower.contains("implementation agent changed files outside declared target_files")
            || lower.contains("implementation noop requires typed task_coverage evidence")
            || lower
                .contains("implementation agent returned accepted status without changed files")
            || lower.contains("patch is empty and item did not declare idempotent_noop")
            || lower.contains("changed files outside declared ownership")
            || lower.contains("changed undeclared path")
            || lower.contains("patch writes undeclared path")
            || lower.contains("declares no target ownership")
            || (lower.contains("write target") && lower.contains("is unsafe"))
            || semantic_verification_blocker(&lower)
            || lower.contains("output not usable")
            || lower.contains("malformedoutput")
            || lower.contains("invalid branch result")
            || is_size_policy_error(&lower))
}

pub(super) fn semantic_verification_blocker(lower_error: &str) -> bool {
    lower_error.contains("verification")
        && lower_error.contains("blocked")
        && (lower_error.contains("agent output") || lower_error.contains("failed verification"))
}

pub(super) fn is_size_policy_error(lower_error: &str) -> bool {
    lower_error.contains("exceeds max")
        && (lower_error.contains("source file")
            || lower_error.contains("function")
            || lower_error.contains("file "))
}

pub(super) fn truncate_for_result(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for ch in value.chars().take(max_chars) {
        output.push(ch);
    }
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

pub(super) fn sanitize_v2_path_segment(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}
