//! Did a write branch leave real work on disk, and how that is recorded.
//!
//! Split from `worktree_branch_a.rs` to hold the 500-line ceiling.
use super::*;

/// Record that a schema-repair failure nonetheless left a real patch on disk.
///
/// A write branch whose schema repair failed produced NO verdict on the work —
/// but the work may still have landed. Two of TDL-020's three attempts died
/// exactly this way, and charging them to the task discarded a patch that
/// existed. This is the third shape of "an attempt burned by something that
/// says nothing about the work", after the HTTP 520 and the verifier timeout.
///
/// The question is answered against the DECLARED BASELINE, never by asking the
/// worktree whether any files changed. Stray tool output, a partial write, or a
/// worktree dirtied by something other than the patch all answer "yes" to the
/// cheap question, and each would refund an attempt that produced nothing.
///
/// **Marking only.** The budget decision lives in the prelude's
/// `remediationBudget`, bounded to once per task. That bound is the safety
/// argument: schema repair already retries under its own cap, so an unbounded
/// exemption trades a burned attempt for a hung task — strictly worse.
///
/// # What this does NOT do
///
/// It does not preserve the patch. A schema failure classifies as `Contract`,
/// which yields `NeedsReview`, and `capture_worktree_branch_manifest` captures
/// only `Accepted`/`Noop` — so the patch is never turned into a manifest and
/// never reaches the canonical repo. It is stranded in the branch worktree and
/// discarded with it.
///
/// **The refunded attempt therefore starts clean and redoes the work.** Seeing a
/// task visibly repeat itself on this path is expected, not a bug.
///
/// So this buys a retry, not a rescue: it stops a malformed *report* from
/// spending the task's budget. The spec's "re-verify the existing patch rather
/// than re-running the round" is not achievable here — there is no surviving
/// patch to re-verify. Making that true would mean capturing a manifest from a
/// non-accepted branch, which touches the write coordinator's safety model and
/// is deliberately out of scope.
/// Did this branch leave real work on disk, measured against the DECLARED
/// BASELINE?
///
/// Never asks the worktree whether any files changed. Stray tool output, a
/// partial write, or a worktree dirtied by something other than the patch all
/// answer "yes" to the cheap question. Fails CLOSED: if the patch cannot be
/// captured we cannot prove work landed, so the answer is `false`.
pub(super) fn worktree_patch_landed(prepared: &PreparedWorktreeBranch) -> bool {
    capture_patch(
        &prepared.workspace,
        &prepared.coordinator_plan.target_files,
        &prepared.baseline,
    )
    .is_ok_and(|captured| !captured.changed_files.is_empty() || !captured.created_files.is_empty())
}

/// Record on EVERY write branch whether a patch landed.
///
/// `patch_landed` is the general predicate: it is set for accepted, rejected
/// and failed branches alike, so a consumer can ask "did this call change
/// anything?" without having to infer it from a status that answers a different
/// question. Three rejection paths that all land nothing — schema-repair
/// exhaustion, a wholesale size-policy rejection, and an ownership violation —
/// are indistinguishable by status but identical here.
///
/// Its first consumer is the prelude's `remediateFindings`, which used to fire
/// a verifier unconditionally after every fix. Observed live on TDL-041: a fix
/// failed host validation at 09:09:55.153 and a verifier started against
/// unchanged code **85.8 ms later**, then returned the same findings. A status
/// check would not have caught it, and would also have waved through an
/// accepted no-op, which likewise leaves the reviewed code untouched.
///
/// `schema_repair_patch_landed` is kept as the narrower marker that
/// `remediationBudget` reads for its once-per-task attempt refund.
///
/// # Scope: worktree writes only
///
/// There are three write modes — `Serial`, `Coordinated`, `Worktree` — and this
/// is the worktree branch runner, so **coordinated and serial writes carry no
/// `patch_landed` marker**. That is total coverage for the only consumer today,
/// and deliberately so rather than by luck:
///
/// - every write the v3 prelude can request is `write: "worktree"` (both
///   `agent()` and `agents()`), which is the sole source of the remediation
///   fixes the gate exists to judge;
/// - the host never silently downgrades. `workflow_live_v2_write.rs`'s
///   `(_, false)` arm ERRORS when worktree isolation is unavailable instead of
///   falling back, so a worktree request cannot quietly become a serial one.
///
/// The prelude-side test `every_write_the_prelude_requests_is_a_worktree_write`
/// fails if that first premise ever stops holding. A consumer reading this
/// marker on a coordinated or serial branch will see it ABSENT, which
/// `landedNothing` deliberately reads as "run the check" — the old behaviour,
/// not a silent skip.
pub(super) fn mark_patch_landed(
    result: &mut WorkflowV2Result,
    prepared: &PreparedWorktreeBranch,
    landed: bool,
    schema_repair_failed: bool,
) {
    if let Some(data) = result.data.as_object_mut() {
        data.insert("patch_landed".to_string(), serde_json::Value::Bool(landed));
    }
    if !schema_repair_failed || !landed {
        return;
    }
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            "schema_repair_patch_landed".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    // Typed gap so "was exempted" and "used the exemption" stay separable in the
    // records rather than having to be inferred from attempt counts later.
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "schema_repair_exempted_{}",
            sanitize_v2_path_segment(&prepared.branch.id)
        ),
        description: format!(
            "schema repair failed for branch '{}', but a patch landed against the declared \
             baseline, so the attempt did real work and produced no verdict. The patch is NOT \
             preserved (a NeedsReview branch is never captured), so the refunded attempt redoes \
             the work from a clean worktree. Refunded ONCE for this task — a second such failure \
             is charged normally.",
            prepared.branch.id,
        ),
        severity: Some("info".to_string()),
    });
}

/// Keyed on the runtime's own error text for the bounded-retry exhaustion, which
/// is the only place this phrasing is produced (`write_errors.rs:213`).
pub(super) fn is_schema_repair_failure_result(result: &WorkflowV2Result) -> bool {
    result
        .data
        .get("error")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|error| error.contains("schema repair failed"))
}
