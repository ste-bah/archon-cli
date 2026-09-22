//! The test baseline at the VERIFICATION base, established before a focused
//! verification fanout dispatches its items (Issue-70).
//!
//! # The gap this closes
//!
//! A focused verifier runs the task's declared filter at the current head of
//! the canonical checkout, but the only baseline record for the task came
//! from its implementation wave, at the task's original base commit. Every
//! commit landed in between — other tasks' work — can turn a test in one of
//! THEIR files red. That test is on nobody's other-owner list, so
//! `verification::enforce_baseline_tests` demotes the accepted verdict, the
//! task is remediated for a failure it cannot fix, and the run loops
//! remediate → verify. Live on wf-0ddadd81, docs-only TASK-TRADING-001
//! failed verification twice on three tests in other tasks' files.
//!
//! # What happens instead
//!
//! Before the items go to the scheduler, each item's declared commands are
//! run at the verification base — the head of the checkout the verifier is
//! told to `cd` into — through the very machinery the implementation waves
//! use ([`super::test_baseline_wave::establish_wave`]): verdicts are cached
//! per (commit, command) so tasks verified at the same head share runs, a
//! red test in a file another task declares is routed to that task as a
//! finding (what feeds its remediation) and listed for this verifier to
//! ignore, and a red test in the task's own file stays its obligation. The
//! record is persisted under the verification call id, beside — never over —
//! the implementation wave's, and the item's `baseline_tests` stamp is
//! rewritten from the record at that commit.
//!
//! A checkout git cannot read a head from leaves the items as they were:
//! the earlier stamp, and the rule as before.

use std::path::Path;

use super::test_baseline_wave::{BranchBaselineRequest, WaveBaselineContext, establish_wave};
use crate::agent_dispatch_port::{WorkflowAgentDispatch, declared_focused_tests};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::verification::baseline_rule::{
    is_focused_verification_call, stamp_baseline_tests_input_at,
};
use crate::v2::{WorkflowV2FanoutItem, WorkflowV2ResultStore};

pub struct VerificationBaselineContext<'a> {
    pub store: &'a WorkflowV2ResultStore,
    pub dispatch: &'a dyn WorkflowAgentDispatch,
    pub universe: Option<&'a WorkflowV2TaskUniverse>,
    /// The fanout call id; the record's stage.
    pub call_id: &'a str,
    /// The checkout the verifiers run their commands in.
    pub repository_root: &'a Path,
    pub parallelism: usize,
}

/// Establish every item's baseline at the head of `repository_root` and
/// re-stamp `baseline_tests` on each. Returns the verification base commit
/// when the call is a focused verification and the head could be read;
/// `None` leaves the items untouched.
pub async fn establish_verification_baseline(
    ctx: &VerificationBaselineContext<'_>,
    items: &mut [WorkflowV2FanoutItem],
) -> Option<String> {
    if !is_focused_verification_call(ctx.call_id) || items.is_empty() {
        return None;
    }
    let base_commit = match crate::repository_record::git_head(ctx.repository_root) {
        Ok(sha) => sha,
        Err(error) => {
            eprintln!(
                "baseline tests: verification base of {} unreadable, keeping the implementation \
                 baseline: {error}",
                ctx.repository_root.display()
            );
            return None;
        }
    };
    let requests: Vec<BranchBaselineRequest> = items
        .iter()
        .filter_map(|item| baseline_request(ctx, item))
        .collect();
    if !requests.is_empty() {
        establish_wave(
            &WaveBaselineContext {
                store: ctx.store,
                dispatch: ctx.dispatch,
                universe: ctx.universe,
                stage_id: ctx.call_id,
                base_commit: &base_commit,
                parallelism: ctx.parallelism,
            },
            &requests,
        )
        .await;
    }
    for item in items.iter_mut() {
        stamp_baseline_tests_input_at(ctx.call_id, ctx.store, &mut item.input, Some(&base_commit));
    }
    Some(base_commit)
}

/// The request for one item, filled the way the implementation wave fills
/// its own (`worktree_wave_prepare::baseline_request`): the item's canonical
/// task ids, its declared focused commands verbatim, the tree the verifier
/// runs in, its declared targets, and its tasks' forbidden paths. `None` for
/// an item naming no task: nothing to baseline it for.
fn baseline_request(
    ctx: &VerificationBaselineContext<'_>,
    item: &WorkflowV2FanoutItem,
) -> Option<BranchBaselineRequest> {
    let source = item.input.get("item").unwrap_or(&item.input);
    let task_ids = crate::v2::review_findings::task_ids_of(source);
    if task_ids.is_empty() {
        return None;
    }
    let forbidden = ctx
        .universe
        .map(|universe| super::forbidden_paths::forbidden_paths(universe, &task_ids))
        .unwrap_or_default();
    Some(BranchBaselineRequest {
        branch_id: item.id.clone(),
        task_ids,
        commands: declared_focused_tests(&item.input),
        worktree: ctx.repository_root.to_path_buf(),
        targets: crate::v2::call_data::target_files_from_value(source),
        forbidden,
    })
}
