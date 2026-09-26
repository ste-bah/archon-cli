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
//! forbidden list. Live, two tasks declare the same command module as a
//! target while their forbidden prose says "all other arms of that file"
//! — a distinction no path matcher can express. So the DECLARATION wins:
//! a path the item declares (a target file, or under a declared directory
//! scope) is never forbidden for that item, because the authored
//! declaration is the more specific statement; the overlap is recorded
//! once per branch as a review gap (`forbidden_declared_conflict_<item>`)
//! so the task wording gets looked at, and the preamble says so. Granted
//! (undeclared) and contested paths stay subject to the rule. This does
//! not reopen the live defect: none of the four files changed there was a
//! declared target, and the scope grant never declares a forbidden path.
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
///
/// A MULTI-task item (cross-task remediation: one write over several tasks'
/// files) drops every pattern whose whole extent lies inside a declared
/// target or directory scope of ANY of its own tasks: a task forbidding
/// exactly its sibling's own file ("`<sibling file>` (<sibling> scope)")
/// must not forbid the item that file. A pattern that reaches beyond the
/// declared paths (a sibling's directory, a basename, a glob) is KEPT: it
/// still freezes every undeclared path it names, and the declared paths in
/// it are exempt at the tool guard and at capture ("declaration wins",
/// above). Built here, the one list the preamble, the tool guard stamp and
/// the capture backstop all read, so the three agree. A single-task item
/// keeps its list as written.
pub(super) fn forbidden_paths(
    task_universe: &WorkflowV2TaskUniverse,
    task_ids: &[String],
) -> ForbiddenPaths {
    let own = || {
        task_universe
            .tasks
            .iter()
            .filter(|task| task_ids.contains(&task.canonical_task_id))
    };
    let forbidden =
        ForbiddenPaths::from_entries(own().flat_map(|task| task.files_forbidden_to_change.iter()));
    if own().count() < 2 {
        return forbidden;
    }
    forbidden.without_within(own().flat_map(|task| {
        task.files_expected_to_change
            .iter()
            .chain(&task.shared_append_target_files)
            .filter_map(|entry| crate::v2::script::declared_path(entry))
    }))
}

/// [`forbidden_paths`] for one branch item. An escalated remediation round
/// (Issue-107) lifts nothing but its exact blocker files: the union of its
/// tasks' lists loses only the patterns wholly inside one of those files,
/// and only files one of its tasks declares, so a directory, a basename or a
/// glob another task forbids (`docs/`, `.mcp.json`, ...) always stays.
pub(super) fn forbidden_paths_for_item(
    task_universe: &WorkflowV2TaskUniverse,
    task_ids: &[String],
    item: &serde_json::Value,
) -> ForbiddenPaths {
    let Some(blockers) = item
        .get("escalation_blocker_paths")
        .and_then(serde_json::Value::as_array)
    else {
        return forbidden_paths(task_universe, task_ids);
    };
    let own = || {
        task_universe
            .tasks
            .iter()
            .filter(|task| task_ids.contains(&task.canonical_task_id))
    };
    let declared: Vec<String> = own()
        .flat_map(|task| {
            task.files_expected_to_change
                .iter()
                .chain(&task.shared_append_target_files)
                .filter_map(|entry| crate::v2::script::declared_path(entry))
        })
        .collect();
    let lift: Vec<String> = blockers
        .iter()
        .filter_map(serde_json::Value::as_str)
        .filter(|path| !path.ends_with('/') && !path.contains('*'))
        .filter(|path| {
            declared.iter().any(|entry| {
                let entry = entry.trim_end_matches('/');
                entry == *path
                    || entry.ends_with(&format!("/{path}"))
                    || path.starts_with(&format!("{entry}/"))
            })
        })
        .map(str::to_string)
        .collect();
    ForbiddenPaths::from_entries(own().flat_map(|task| task.files_forbidden_to_change.iter()))
        .without_within(lift)
}

/// The sentence appended to the branch's task after the scope roots one.
/// Empty when nothing is forbidden.
pub(super) fn preamble(forbidden: &ForbiddenPaths) -> String {
    if forbidden.is_empty() {
        return String::new();
    }
    format!(
        "\nForbidden paths for this task (never edit; a needed change there is a residual gap \
         to report, not an edit to make; declared targets take precedence): {}.\n",
        forbidden.describe()
    )
}

/// Gap id prefix for a branch whose declared targets the forbidden list
/// also names.
pub(crate) const FORBIDDEN_DECLARED_CONFLICT_GAP_PREFIX: &str = "forbidden_declared_conflict_";

/// Record, whatever the branch's status, the declared targets the task
/// text also forbids: the declaration was honoured, and a reviewer has to
/// be told the task contradicts itself. Silent when there is no overlap.
pub(super) fn report_forbidden_declared_conflict(
    result: &mut WorkflowV2Result,
    branch_id: &str,
    conflicting: &[String],
) {
    if conflicting.is_empty() {
        return;
    }
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "{FORBIDDEN_DECLARED_CONFLICT_GAP_PREFIX}{}",
            sanitize_v2_path_segment(branch_id)
        ),
        description: truncate_for_result(
            &format!(
                "write item '{branch_id}' has {} path(s) declared and forbidden at once by the \
                 task text; the declaration was honoured — check the task wording: {}",
                conflicting.len(),
                conflicting.join(", ")
            ),
            1_000,
        ),
        severity: Some("review".to_string()),
    });
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            "forbidden_declared_conflict".to_string(),
            serde_json::json!(conflicting),
        );
    }
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
