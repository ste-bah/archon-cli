use super::*;

/// The substring every empty-reply producer must keep in its message.
///
/// Two crates raise this condition — `WorkflowV2AgentError::EmptyReply` here
/// and the subagent stream round in `archon-core` — and three sites below
/// classify on it. A constant plus `empty_reply_marker_matches_producers`
/// means a reworded message fails a test instead of silently downgrading a
/// transport failure into "invalid implementation evidence".
pub(super) const EMPTY_REPLY_MARKER: &str = "empty reply";

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
        summary: if error.contains(EMPTY_REPLY_MARKER) {
            format!("write branch '{item_id}' received no usable provider reply; implementation was not evaluated")
        } else {
            format!("write branch '{item_id}' produced invalid implementation evidence after repair")
        },
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

/// A branch that returned an error the write layer has no classification for.
///
/// Same typed remediation shape as [`write_branch_validation_error_result`] —
/// the same gap id, the same `write_branch_error_kind` classification, the same
/// scope-expansion extraction — because the consumer (workflow.js, and the
/// aggregation in `result.rs`) reads one shape, not two. Only the summary and
/// one marker differ, so "the write layer recognised this rejection" and "the
/// write layer did not recognise this error and scoped it to the branch anyway"
/// stay separable in the records instead of having to be inferred later.
///
/// Reached only from `worktree_wave::worktree_wave_outcomes`, and only for
/// errors that are NOT in that function's explicit fatal list.
pub(super) fn write_branch_unhandled_error_result(
    item_id: &str,
    input: Option<&serde_json::Value>,
    error: &str,
) -> WorkflowV2Result {
    let mut result = write_branch_validation_error_result(item_id, input, error);
    if !error.contains(EMPTY_REPLY_MARKER) {
        result.summary = format!("write branch '{item_id}' failed with an error the write layer does not classify");
    }
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            "branch_error_unclassified".to_string(),
            serde_json::Value::Bool(true),
        );
    }
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
        || lower.contains("outside declared ownership")
        // `WorkflowV2WriteSafetyError::UnsafeTarget` ("write target '<path>' for
        // item '<id>' is unsafe") is the phrasing the ownership guards actually
        // produced live, and it reaches here bare — `project_artifacts.rs` and
        // `write_mode_paths.rs` raise it without the "outside declared
        // target_files" prefix. Both `write_branch_error_kind` and
        // `is_write_branch_validation_error` already recognise it as a scope
        // rejection; this predicate did not, so the actionable
        // `scope_expansion_needed_*` gap below was never emitted for the very
        // failure it was written for.
        || (lower.contains("write target") && lower.contains("is unsafe"));
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

pub(super) fn canonical_task_ids_from_write_error_input(
    input: Option<&serde_json::Value>,
) -> Vec<String> {
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

/// A branch that was INTERRUPTED rather than one that did the work badly.
///
/// Kept apart from the validation and unhandled results because the difference
/// is what happens next: this is `NeedsReview` with the branch's evidence
/// retained, so the wave survives and the work can be re-asked. A branch
/// stopped by the host reaching a limit has said nothing about whether its
/// implementation was right.
pub(super) fn write_branch_interrupted_result(
    item_id: &str,
    input: &serde_json::Value,
    error: &str,
) -> WorkflowV2Result {
    let source = input.get("item").unwrap_or(input);
    let canonical_task_ids = canonical_task_ids_from_generated_value(source, None);
    let contention = is_host_resource_contention(error);
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: if contention {
            format!(
                "write branch '{item_id}' could not start because a host resource was still held                  by an earlier run of it"
            )
        } else {
            format!("write branch '{item_id}' timed out before returning usable output")
        },
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "write branch interruption was retained as item-level remediation data for workflow.js",
    ));
    // The gap id stays `write_branch_timeout_*` for a contention case too. It
    // is a key, not a description: the authored script and the aggregation in
    // `result.rs` match on it, and splitting it by cause would make the new
    // cause invisible to every consumer that already handles this class. What
    // distinguishes them is `branch_host_resource_contention` below.
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
        "branch_host_resource_contention": contention,
        "failure_kind": BranchFailureKind::Contract,
        "error": truncate_for_result(error, 2_000),
    });
    result
}

pub(super) fn failure_kind_from_write_result(
    result: &WorkflowV2Result,
) -> Option<BranchFailureKind> {
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

/// Marker phrase shared by the branch loop and the predicate below.
///
/// Both sides name the same constant rather than one of them matching prose the
/// other happens to write, because a message reworded on one side and matched on
/// the other is a silent behaviour change: the branch would stop being
/// recoverable and nobody would see it in the diff.
pub(super) const CALL_TIME_BUDGET_EXHAUSTED: &str = "exhausted its total time budget";

/// Errors that mean the branch was STOPPED, not that its work was wrong.
///
/// Named for the class rather than for timeouts, which is all it used to hold.
/// Everything here shares one property: the branch never got to say whether its
/// implementation was correct, so discarding the wave over it throws away work
/// that no evidence has faulted.
pub(super) fn is_recoverable_write_branch_interruption(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("subagent timed out")
        || lower.contains("timed out after")
        // Running out of the budget for the WHOLE call is the same kind of
        // event as one dispatch timing out: the work is unfinished, nothing is
        // wrong with the branch itself, and the evidence gathered so far is
        // worth keeping. Treated as a hard error it would take the wave with it
        // and discard what the branch had learned.
        || lower.contains(CALL_TIME_BUDGET_EXHAUSTED)
        || is_host_resource_contention(error)
}

/// A host-side resource the branch could not take, rather than anything the
/// branch did.
///
/// Observed live as the error that ended a fifteen-task run at its third task:
/// a killed branch left its subagent registration in `Running`, and the retry —
/// which re-dispatches under the SAME id — was refused with "subagent already
/// exists and is running". The leak itself is fixed where it belongs, by making
/// that registration release on drop. This predicate is the second half: the
/// message arrived here wrapped in "agent transport failed", which routed it
/// past every validation branch into the unclassified result, and an
/// unclassified result is terminal. So a resource collision — a class that can
/// arise again from any concurrent holder — permanently failed a task whose
/// implementation nothing had examined.
pub(super) fn is_host_resource_contention(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    // Matched WITH the word "subagent", not on the bare phrase. This predicate
    // runs before the validation classifier, so anything it captures is treated
    // as an interruption and re-askable — and an agent is free to write
    // "already exists and is running" in its own summary, which can reach here
    // inside a validation error. Requiring the noun keeps the match to the
    // registry's own wording.
    lower.contains("subagent already exists and is running")
        || lower.contains("max concurrent subagents reached")
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

/// Turn one branch's dispatch `Result` into the branch outcome it deserves.
///
/// Lives beside the predicates it consults rather than beside the loop that
/// calls it: this is entirely classification — which of "stopped", "judged
/// wrong", or "not ours to classify" an error is — and the loop has no say in
/// it. Keeping them apart is what stopped a classification change from having
/// to be read against dispatch control flow to be understood.
/// Takes the branch's id and input rather than the branch itself, which is what
/// lets this live here at all: `WorktreeBranchExecution` is private to the
/// dispatch module, and reaching for it from a sibling would have meant
/// widening a type's visibility to satisfy a classifier that never needed the
/// type. Two values are all the classification reads.
pub(super) fn normalize_worktree_agent_result(
    result: crate::WorkflowResult<WorkflowV2Result>,
    branch_id: &str,
    input: &serde_json::Value,
) -> crate::WorkflowResult<WorkflowV2Result> {
    match result {
        Ok(result) => Ok(result),
        Err(err) if is_recoverable_write_branch_interruption(&err.to_string()) => Ok(
            write_branch_interrupted_result(branch_id, input, &err.to_string()),
        ),
        Err(err) if is_write_branch_validation_error(&err.to_string()) => Ok(
            write_branch_validation_error_result(branch_id, Some(input), &err.to_string()),
        ),
        Err(err) => Err(err),
    }
}

/// Mark an outcome as one that stopped because it stopped getting CLOSER.
///
/// Additive on purpose. The underlying rejection is still the most actionable
/// thing in the record — it names the file and the cap — so it is left exactly
/// as it is. What was missing is why the branch stopped: a record reading only
/// "produced invalid implementation evidence after repair" describes one bad
/// answer, when what actually happened is that the branch was re-asked
/// `attempts` times and never improved. Those call for different remediation —
/// one is a fix, the other is a change of scope or approach — and a consumer
/// that cannot tell them apart will keep prescribing the first.
///
/// A stall is reported, never inferred: only the loop knows it re-asked and got
/// nowhere, and no amount of reading the error text recovers that.
pub(super) fn stamp_no_progress(
    result: crate::WorkflowResult<WorkflowV2Result>,
    attempts: usize,
) -> crate::WorkflowResult<WorkflowV2Result> {
    let mut outcome = match result {
        Ok(outcome) => outcome,
        // An error the classification declined to own stays declined. Stamping
        // it would claim a diagnosis this function did not make.
        Err(err) => return Err(err),
    };
    if let Some(data) = outcome.data.as_object_mut() {
        data.insert(
            "branch_no_progress".to_string(),
            serde_json::Value::Bool(true),
        );
        data.insert(
            "branch_attempts".to_string(),
            serde_json::Value::from(attempts),
        );
    }
    outcome.summary = format!(
        "{} — it stopped getting closer after {attempts} re-ask(s)",
        outcome.summary
    );
    Ok(outcome)
}
