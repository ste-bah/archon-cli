use archon_session::plan::PlanStepStatus;

use super::self_calibration::plan_step_counts;

#[test]
fn failed_steps_are_blocked_and_excluded_from_planning_accuracy() {
    let (completed, skipped, blocked) = plan_step_counts([
        PlanStepStatus::Complete,
        PlanStepStatus::Skipped,
        PlanStepStatus::Failed,
        PlanStepStatus::Pending,
        PlanStepStatus::InProgress,
    ]);

    assert_eq!(completed, 1);
    assert_eq!(skipped, 1);
    assert_eq!(blocked, 3);
    assert_eq!((completed + skipped) as f32 / 5.0, 0.4);
}
