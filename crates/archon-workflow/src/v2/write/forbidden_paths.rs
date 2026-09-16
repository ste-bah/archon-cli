//! A write branch's forbidden paths: resolved from its tasks, told to the
//! agent, stamped for the tool guard, and enforced at capture (Issue-30).
//!
//! # The gap this closes
//!
//! `task_universe_parsing` fills `files_forbidden_to_change` for every task
//! and, until now, nothing consumed it. The ownership grant
//! (`worktree_scope_grant`, Issues 16/27) admits any unclaimed change inside
//! the plan's scope roots, and a forbidden file in the task's own crate is
//! exactly that. Live on wf-719ff3b0 `agents-14-1`: the task's forbidden list
//! named the crate's gate module, `coverage.rs`, `data_lake.rs`,
//! `data_store.rs` and `validation.rs`; the coder edited the gate module, the
//! coverage module, a data-lake contract file and a data-store test, all four
//! were granted as unclaimed in-scope changes, declared in the manifest and
//! committed under the task.
//!
//! # Three readers, one list
//!
//! - The PREAMBLE: the list is appended to the branch's task after the scope
//!   roots sentence, so the agent is told the rule the gate will apply.
//! - The TOOL GUARD: the wire patterns are stamped onto the branch input
//!   under [`FORBIDDEN_PATHS_INPUT_KEY`]; the host dispatch reads them back
//!   (`agent_dispatch_port::declared_forbidden_paths`) and scopes them for
//!   `archon_tools::workflow_read_guard`, which refuses a Write/Edit/patch
//!   call at the file before it changes. A Bash edit cannot be intercepted
//!   there, which is why the third reader exists.
//! - The CAPTURE BACKSTOP: `ScopeGrant::resolve` partitions every remaining
//!   changed path that matches into `ScopeGrant::forbidden`, and
//!   `run_one_worktree_branch` REJECTS the branch before the ownership gates
//!   when that set is non-empty. Nothing is restored or dropped: a
//!   half-reverted edit set breaks the crate for every later verification,
//!   so the patch is simply not captured, the branch goes `needs_review`
//!   with a gap naming the paths, and the partial-work sidecar the wave
//!   keeps for a rejected branch tells the next attempt what to undo.
//!
//! Declared-and-forbidden: a path can be both a declared target and on the
//! forbidden list (an author's contradiction, or a task forbidding a whole
//! directory one of its own targets sits in). Forbidden wins, but only when
//! the path was actually changed — a declared target left untouched is fine.
//! The alternative, declaration wins, would let the exact live defect
//! through whenever the scope grant had already declared the path.
//!
//! The matcher itself is `archon_write_plan::ForbiddenPaths`, shared with the
//! tool guard so both layers answer identically.

pub(super) use archon_write_plan::ForbiddenPaths;

use super::errors::{
    branch_validation_failure_fields, sanitize_v2_path_segment, truncate_for_result,
};
use crate::agent_dispatch_port::FORBIDDEN_PATHS_INPUT_KEY;
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::{BranchFailureKind, WorkflowV2Evidence, WorkflowV2ResidualGap, WorkflowV2Result};

/// Gap id prefix for a branch rejected for changing a forbidden path.
pub(crate) const FORBIDDEN_PATH_CHANGED_GAP_PREFIX: &str = "forbidden_path_changed_";

/// The union of `files_forbidden_to_change` over the branch's canonical
/// tasks, normalised once. Empty when no task declares any.
pub(super) fn forbidden_paths(
    task_universe: &WorkflowV2TaskUniverse,
    task_ids: &[String],
) -> ForbiddenPaths {
    ForbiddenPaths::from_entries(
        task_universe
            .tasks
            .iter()
            .filter(|task| task_ids.contains(&task.canonical_task_id))
            .flat_map(|task| task.files_forbidden_to_change.iter()),
    )
}

/// The sentence appended to the branch's task after the scope roots one.
/// Empty when nothing is forbidden.
pub(super) fn preamble(forbidden: &ForbiddenPaths) -> String {
    if forbidden.is_empty() {
        return String::new();
    }
    format!(
        "\nForbidden paths for this task (never edit; a needed change there is a residual gap \
         to report, not an edit to make): {}.\n",
        forbidden.describe()
    )
}

/// Stamp the wire patterns onto the branch input for the host dispatch to
/// hand the tool guard. A top-level key, like the artifact policy stamp:
/// the top level is host-built and never rendered to the agent, and the key
/// is in `reuse_identity::VOLATILE_INPUT_KEYS` so it never moves the reuse
/// hash. Absent when nothing is forbidden.
pub(super) fn stamp(input: &mut serde_json::Value, forbidden: &ForbiddenPaths) {
    if forbidden.is_empty() {
        return;
    }
    if let Some(object) = input.as_object_mut() {
        object.insert(
            FORBIDDEN_PATHS_INPUT_KEY.to_string(),
            serde_json::json!(forbidden.patterns()),
        );
    }
}

/// The branch result for a worktree that changed forbidden paths: the same
/// typed shape `write_branch_validation_error_result` gives a semantic
/// rejection — `needs_review`, a review-severity gap, `failure_kind` and
/// `branch_error_from_runtime` in `data` — with the paths named for the
/// reviewer and for the retry preamble, and `patch_landed` false because
/// nothing was captured.
pub(super) fn forbidden_rejection_result(
    item_id: &str,
    canonical_task_ids: &[String],
    changed: &[String],
) -> WorkflowV2Result {
    let failure_kind = BranchFailureKind::Semantic;
    let (status, evidence_kind, severity) = branch_validation_failure_fields(&failure_kind);
    let summary = format!(
        "write item '{item_id}' changed {} path(s) the task forbids: {}; the patch was not \
         captured",
        changed.len(),
        changed.join(", ")
    );
    let mut result = WorkflowV2Result {
        status,
        summary: truncate_for_result(&summary, 2_000),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        evidence_kind,
        "write branch changed paths its task's Files Forbidden to Change list names; the \
         rejection was retained as typed remediation data",
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "{FORBIDDEN_PATH_CHANGED_GAP_PREFIX}{}",
            sanitize_v2_path_segment(item_id)
        ),
        description: truncate_for_result(
            &format!(
                "write item '{item_id}' changed {} path(s) its task forbids ({}); the patch was \
                 not captured and nothing landed. Restore each of these to the baseline, keep \
                 the rest of the work, and report any change they needed as a residual gap.",
                changed.len(),
                changed.join(", ")
            ),
            1_000,
        ),
        severity: Some(severity.to_string()),
    });
    result.data = serde_json::json!({
        "branch_id": item_id,
        "item_id": item_id,
        "canonical_task_ids": canonical_task_ids,
        "branch_error_from_runtime": true,
        "failure_kind": failure_kind,
        "error": truncate_for_result(&summary, 2_000),
        "forbidden_paths_changed": changed,
        "patch_landed": false,
    });
    result
}

#[cfg(test)]
#[path = "forbidden_paths_tests.rs"]
mod tests;
