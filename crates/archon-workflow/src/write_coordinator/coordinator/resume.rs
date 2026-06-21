use crate::work_unit_coverage::CoverageVerdict;
use crate::work_unit_gate;
use crate::write_coordinator::patch_apply::{ApplyResumeStatus, resume_status};
use crate::write_coordinator::patch_manifest::ManifestStatus;

use super::{CoordinatedOutcome, FanoutCtx, PlanInput};

pub(super) fn filter_resumable_items(
    ctx: &FanoutCtx<'_>,
    plans_input: Vec<PlanInput>,
) -> (Vec<PlanInput>, CoordinatedOutcome) {
    let mut outcome = CoordinatedOutcome::new(&ctx.run.id, &ctx.stage.id);
    let mut pending = Vec::new();
    let mut accepted_inputs = Vec::new();
    for input in plans_input {
        match resume_status(&input.item.id, &ctx.run_root, &ctx.stage.id) {
            ApplyResumeStatus::Applied if resumable_item_has_current_coverage(ctx, &input) => {
                outcome
                    .item_status
                    .insert(input.item.id.clone(), ManifestStatus::Applied);
                accepted_inputs.push(input);
            }
            ApplyResumeStatus::IdempotentNoop
                if resumable_item_has_current_coverage(ctx, &input) =>
            {
                outcome
                    .item_status
                    .insert(input.item.id.clone(), ManifestStatus::IdempotentNoop);
                accepted_inputs.push(input);
            }
            ApplyResumeStatus::Conflicted => {
                outcome
                    .item_status
                    .insert(input.item.id.clone(), ManifestStatus::Conflicted);
            }
            _ => pending.push(input),
        }
    }
    if !stage_resume_coverage_accepted(ctx, &accepted_inputs) {
        for input in accepted_inputs {
            outcome.item_status.remove(&input.item.id);
            pending.push(input);
        }
    }
    (pending, outcome)
}

fn resumable_item_has_current_coverage(ctx: &FanoutCtx<'_>, input: &PlanInput) -> bool {
    let required = work_unit_gate::required_for_item(ctx.stage, &input.item.payload);
    if required.is_empty() {
        return true;
    }
    let mut bundles = crate::work_unit_coverage::bundles_from_agent_records(
        &ctx.run_root,
        &ctx.stage.id,
        [input.item.id.clone()],
    );
    let coverage = work_unit_gate::evaluate_required(
        ctx.run,
        ctx.stage,
        required,
        bundles.remove(&input.item.id).unwrap_or_default(),
    );
    coverage.verdict == CoverageVerdict::Accepted
}

fn stage_resume_coverage_accepted(ctx: &FanoutCtx<'_>, inputs: &[PlanInput]) -> bool {
    let payloads = inputs
        .iter()
        .map(|input| (input.item.id.clone(), input.item.payload.clone()));
    work_unit_gate::evaluate_agent_records(ctx.run, ctx.stage, payloads, &ctx.run_root)
        .is_none_or(|coverage| coverage.verdict == CoverageVerdict::Accepted)
}
