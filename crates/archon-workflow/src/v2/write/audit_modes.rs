//! Audited writes share capture/apply enforcement regardless of scheduling mode.
use super::*;

pub(super) async fn run(
    ctx: WriteFanoutContext<'_>,
    branches: Vec<WorkflowV2FanoutItem>,
    mut plan: WorkflowV2WritePlan,
    reused: Vec<WorkflowV2Result>,
) -> WorkflowResult<WorkflowV2Result> {
    // Preserve the planner's serial/coordinated wave boundaries. Only the
    // workspace/apply mechanism changes: no audited writer mutates canonical
    // source before its actual patch has passed the audit gate.
    for wave in &mut plan.waves {
        for assignment in &mut wave.assignments {
            assignment.worktree_path = Some(
                ctx.v2_store
                    .root()
                    .join("worktrees")
                    .join(sanitize_v2_path_segment(&ctx.execution.call.id))
                    .join(sanitize_v2_path_segment(&assignment.item_id))
                    .display()
                    .to_string(),
            );
        }
    }
    run_worktree_v2_write_fanout(ctx, branches, plan, reused).await
}
