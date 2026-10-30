//! Finish one command's result after process teardown.
use super::*;

pub(super) fn teardown_fault(evidence: &str) -> Option<super::super::faults::FaultCause> {
    evidence
        .starts_with(super::super::faults::TEARDOWN_UNVERIFIED)
        .then_some(super::super::faults::FaultCause::TeardownUnverified)
}

type CapturedStream = (Vec<u8>, bool);

pub(super) fn finish(
    mut result: CheckResult,
    site: &CommandSite<'_>,
    command: &crate::acceptance_world::AuthorizedCommand,
    pipes: &(CapturedStream, CapturedStream),
    process_tree_gone: bool,
    moved: Option<&[super::mutation_fd::MoveOutcome]>,
    parent_snapshots: Option<&[Option<Result<String, String>>]>,
) -> WorkflowResult<CheckResult> {
    if let (Some(plan), Some(moved)) = (site.mutation_plan, moved) {
        super::mutation::record_move_outcomes(&mut result, plan, moved);
    }
    if process_tree_gone {
        if let (Some(plan), Some(moved), Some(root)) =
            (site.mutation_plan, moved, site.mutation_root.as_ref())
        {
            super::mutation::record_check_recreated(&mut result, plan, moved, root);
        }
        if let (Some(plan), Some(moved), Some(parent_snapshots), Some(root)) = (
            site.mutation_plan,
            moved,
            parent_snapshots,
            site.mutation_root.as_ref(),
        ) {
            super::mutation::restore_plan(&mut result, plan, moved, parent_snapshots, root);
        }
    } else if let (Some(plan), Some(moved)) = (site.mutation_plan, moved) {
        super::mutation::mark_plan_unsafe(&mut result, plan, moved);
    }
    result.classify_raw(&String::from_utf8_lossy(command.bytes()));
    // Issue 544: every text-dependent fact a later reader needs is computed
    // here from the raw capture; then every stream is masked whole.
    if result.exit_code != Some(0)
        && let Some(classification) = &mut result.classification
        && classification.failure_signature.is_none()
    {
        classification.failure_signature =
            Some(crate::v2::acceptance_regression::failure_signature(
                result.exit_code,
                &String::from_utf8_lossy(&site.placeholder(&result.stderr, pipes.1.1)),
                &String::from_utf8_lossy(&site.placeholder(&result.stdout, pipes.0.1)),
            ));
    }
    let protocol = site.mutation_plan.map(|plan| plan.nonce.as_str());
    result.keep_verdict_streams(protocol);
    result.stdout = site.redact(&result.stdout, pipes.0.1);
    result.stderr = site.redact(&result.stderr, pipes.1.1);
    Ok(result)
}
