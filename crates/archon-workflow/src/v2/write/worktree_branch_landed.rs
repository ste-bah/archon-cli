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
/// It does not turn the patch into a manifest. A schema failure classifies as
/// `Contract`, which yields `NeedsReview`, and `capture_worktree_branch_manifest`
/// captures only `Accepted`/`Noop` — so the patch never reaches the canonical
/// repo on this attempt. Capturing a manifest from a non-accepted branch would
/// touch the write coordinator's safety model and is deliberately out of scope.
///
/// The worktree's diff is NOT lost, though: a branch that ends without a
/// manifest and without acceptance keeps its work as partial work
/// (`partial_work::branch_keeps_partial_work`, at wave collection), and the
/// next attempt at the same canonical tasks starts from it
/// (`partial_work::resume_into_workspace`). That applies equally to the
/// audit-gate rejection (Issue-14), where the work was complete and only the
/// disposition was wrong. What is excluded from resume is a branch whose
/// tasks have since LANDED — its accepted work is already in the baseline.
///
/// So this buys a retry with the diff on disk, not a re-verification of a
/// manifest: the refunded attempt is told what it continues from and must
/// still return an acceptable envelope of its own.
/// Did this branch leave real work on disk, measured against the baseline of
/// the plan it is judged by — the declared targets widened by every granted
/// unclaimed path (`ScopeGrant`)?
///
/// Never asks the worktree whether any files changed. Stray tool output, a
/// partial write, or a worktree dirtied by something other than the patch all
/// answer "yes" to the cheap question. Fails CLOSED: if the patch cannot be
/// captured we cannot prove work landed, so the answer is `false`.
///
/// Judged against the GRANTED plan, not the declared one: a branch whose only
/// change is a granted unclaimed file has a manifest, lands, and must not
/// report `patch_landed: false` to `remediateFindings`. The workspace is
/// rebuilt on the granted plan exactly as `capture_and_validate_worktree_patch`
/// does, because `capture_patch` validates the worktree's changes against
/// `workspace.plan` and would refuse the granted path as undeclared.
///
/// A declared target `.gitignore` covers counts too, when its content moved
/// off the baseline (see [`captured_patch_landed`]). This is the ONE answer:
/// `patch_landed`, `schema_repair_patch_landed` and the delivery receipt are
/// all stamped from it.
pub(super) fn worktree_patch_landed(
    prepared: &PreparedWorktreeBranch,
    grant: &super::worktree_scope_grant::ScopeGrant,
) -> PatchLanding {
    let workspace = ItemWorkspace {
        plan: grant.plan.clone(),
        baseline_commit: prepared.workspace.baseline_commit.clone(),
        materialized_ignored: prepared.workspace.materialized_ignored.clone(),
    };
    workspace_patch_landed(&workspace, &grant.plan.target_files, &prepared.baseline)
}

/// Which half of a branch's work landed. Kept apart because only the tracked
/// half survives into partial work (`partial_work::capture_partial_work`
/// diffs with git, which never sees an ignored path), and the schema-repair
/// refund is only worth spending on work the next attempt resumes from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PatchLanding {
    /// A git-visible change: modified or created against the baseline commit.
    pub(super) tracked: bool,
    /// A declared gitignored deliverable whose bytes moved off the baseline.
    pub(super) ignored: bool,
}

impl PatchLanding {
    pub(super) fn any(self) -> bool {
        self.tracked || self.ignored
    }
}

/// Capture, then judge. Fails CLOSED: a capture error is "nothing landed".
pub(super) fn workspace_patch_landed(
    workspace: &ItemWorkspace,
    targets: &[crate::write_coordinator::NormalizedPath],
    baseline: &crate::write_coordinator::CanonicalBaseline,
) -> PatchLanding {
    capture_patch(workspace, targets, baseline)
        .map(|captured| captured_patch_landed(&captured, baseline))
        .unwrap_or_default()
}

/// Did this capture carry real work: a git-visible change, or a declared
/// gitignored deliverable whose bytes differ from the baseline?
///
/// Ignored deliverables never appear in a git diff — `capture_patch` carries
/// them as bytes in `ignored_files` and the host archives them under the run's
/// `artifacts/ignored-deliverables/` — so reading only the diff stamped a real
/// edit to one `patch_landed: false`, and the post-review loop then skipped
/// its verifier and spent the round on work that was done.
///
/// `ignored_files` holds every ignored target in the plan that EXISTS in the
/// worktree, changed or not: an existing one is materialised from canonical
/// before the agent runs. So presence proves nothing. See
/// [`ignored_deliverable_changed`] for what does.
pub(super) fn captured_patch_landed(
    captured: &CapturedPatch,
    baseline: &crate::write_coordinator::CanonicalBaseline,
) -> PatchLanding {
    PatchLanding {
        tracked: !captured.changed_files.is_empty() || !captured.created_files.is_empty(),
        ignored: captured
            .ignored_files
            .iter()
            .any(|(rel, _)| ignored_deliverable_changed(rel, captured, baseline)),
    }
}

/// Judged against the BASELINE's own record of the path, never against a
/// missing one:
///
/// - no baseline entry — a path granted into the plan after the baseline was
///   sealed, e.g. one the envelope merely listed — is not counted. The
///   capture's pre-hash for it reads "absent", which proves nothing;
/// - the baseline saw a regular file: counted only when the content hash
///   now differs;
/// - the baseline saw nothing there: a true create, counted;
/// - the baseline saw something it could not hash (a symlink, since
///   `file_meta` does not follow one): not counted.
fn ignored_deliverable_changed(
    rel: &str,
    captured: &CapturedPatch,
    baseline: &crate::write_coordinator::CanonicalBaseline,
) -> bool {
    let Some(before) = baseline.declared_target_meta.get(rel) else {
        return false;
    };
    let Some(after) = captured.post_hashes.get(rel).map(|hash| hash.trim()) else {
        return false;
    };
    if after.is_empty() || after == "deleted" {
        return false;
    }
    if !before.exists {
        return true;
    }
    let before = before.blake3_hex.trim();
    !before.is_empty() && before != after
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
    branch_id: &str,
    landed: PatchLanding,
    schema_repair_failed: bool,
) {
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            "patch_landed".to_string(),
            serde_json::Value::Bool(landed.any()),
        );
    }
    // The refund buys a retry that resumes from the kept diff. An ignored
    // deliverable is not in that diff, so it earns `patch_landed` but not this.
    if !schema_repair_failed || !landed.tracked {
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
            sanitize_v2_path_segment(branch_id)
        ),
        description: format!(
            "schema repair failed for branch '{}', but a patch landed against the declared \
             baseline, so the attempt did real work and produced no verdict. The patch is not \
             a manifest, but the worktree diff is kept as partial work and applied to the next \
             attempt at this task. Refunded ONCE for this task — a second such failure is \
             charged normally.",
            branch_id,
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

#[cfg(test)]
#[path = "worktree_branch_landed_tests.rs"]
mod tests;
