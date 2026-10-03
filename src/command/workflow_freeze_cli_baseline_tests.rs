//! An attempt the host kills before the freeze saves its first verdict
//! still leaves a progress line, so the retry's saved work reads as growth.

use crate::command::workflow_host_command_operational::{
    NextStep, OperationalAttempt, next_step, reported_progress,
};

#[test]
fn the_staged_freeze_reports_a_progress_baseline_before_it_builds_anything() {
    let mut stderr = Vec::new();
    let resume = super::staged_freeze_resume(&mut stderr);
    assert_eq!(reported_progress(&stderr), Some(0));
    assert_eq!(resume.progress.total(), 0);
    let attempt = |attempt, progress| OperationalAttempt {
        attempt,
        reason: "timed_out",
        elapsed_secs: 7_800,
        progress,
    };
    assert_eq!(
        next_step(&[attempt(1, Some(0)), attempt(2, Some(3))]),
        NextStep::Retry
    );
    assert_eq!(
        next_step(&[attempt(1, None), attempt(2, Some(3))]),
        NextStep::Pause("no_progress_evidence"),
        "the gap the baseline closes"
    );
}
