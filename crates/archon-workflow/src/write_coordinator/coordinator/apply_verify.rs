//! Apply one validated wave and run its verifier under the repository lock.

use std::collections::BTreeMap;
use std::path::Path;

use super::{FanoutCtx, FanoutError};
use crate::write_coordinator::patch_apply::{
    ApplyRecord, VerifyResult, apply_wave, with_repo_lock,
};
use crate::write_coordinator::patch_manifest::PatchManifest;
use crate::write_coordinator::{ItemId, WaveId};

pub(super) fn apply_and_verify(
    ctx: &FanoutCtx<'_>,
    canonical: &Path,
    wave_id: WaveId,
    manifests: &[PatchManifest],
    pre_by_item: &BTreeMap<ItemId, BTreeMap<String, String>>,
) -> Result<(ApplyRecord, VerifyResult), FanoutError> {
    // Environment construction is an operational precondition. Validate and
    // retain it before any manifest is applied so operator faults pause cleanly.
    let verify_environment = if ctx
        .stage
        .verify_command
        .as_deref()
        .is_some_and(|cmd| !cmd.trim().is_empty())
    {
        Some(
            crate::write_coordinator::patch_apply::verify_environment_ready(&ctx.run_root)
                .map_err(FanoutError::Apply)?,
        )
    } else {
        None
    };
    with_repo_lock(canonical, || {
        let apply_record = apply_wave(
            canonical,
            manifests,
            pre_by_item,
            wave_id,
            &ctx.run_root,
            &ctx.run.id,
            &ctx.stage.id,
        )?;
        let verify = crate::write_coordinator::patch_apply::run_wave_verify_prepared(
            canonical,
            ctx.stage.verify_command.as_deref(),
            wave_id,
            &ctx.run_root,
            &ctx.stage.id,
            verify_environment,
        )?;
        Ok((apply_record, verify))
    })
    .map_err(FanoutError::Apply)
}
