use crate::persistence;
use crate::runner::StageRunOutput;
use crate::write_coordinator::patch_manifest::{CapturedPatch, ManifestStatus, persist_manifest};

use super::{FanoutCtx, ItemState};

pub(super) fn persist(
    ctx: &FanoutCtx<'_>,
    it: &ItemState<'_>,
    captured: &CapturedPatch,
    output: &StageRunOutput,
    reason: &str,
) {
    let _ = persist_manifest(
        &ctx.run_root,
        &ctx.run.id,
        &ctx.stage.id,
        &it.plan.item_id,
        captured,
        ManifestStatus::Failed {
            reason: reason.to_string(),
        },
    );
    let _ = persistence::record_agent_output(
        ctx.store,
        &ctx.run.id,
        &ctx.stage.id,
        &it.plan.item_id,
        Some(output),
        None,
        false,
        Some(reason),
    );
}

pub(super) fn persist_capture_error(
    ctx: &FanoutCtx<'_>,
    it: &ItemState<'_>,
    output: &StageRunOutput,
    reason: &str,
) {
    let captured = empty_failure_capture(it);
    persist(ctx, it, &captured, output, reason);
}

fn empty_failure_capture(it: &ItemState<'_>) -> CapturedPatch {
    let post_hashes = it
        .plan
        .target_files
        .iter()
        .map(|target| (target.as_str(), String::new()))
        .collect();
    CapturedPatch {
        patch_bytes: Vec::new(),
        changed_files: Vec::new(),
        created_files: Vec::new(),
        deleted_files: Vec::new(),
        pre_hashes: Default::default(),
        post_hashes,
        baseline_commit: it.workspace.baseline_commit.clone(),
    }
}
