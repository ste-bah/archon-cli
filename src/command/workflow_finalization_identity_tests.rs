//! The transition rule, stated directly: which persisted outcome a new
//! finalization may replace, and which one it may never contradict.

use super::*;

fn decided(status: WorkflowV2Status) -> FinalizationRecordV1 {
    FinalizationRecordV1::new(WorkflowRunKind::AuthoredTaskWorkflow, status, None)
}

fn undecided(status: RunStatus) -> FinalizationRecordV1 {
    FinalizationRecordV1::for_run_status(WorkflowRunKind::AuthoredTaskWorkflow, status)
}

#[test]
fn a_run_with_no_decided_terminal_status_yet_may_decide_one() {
    for resume_point in [RunStatus::Paused, RunStatus::Cancelled, RunStatus::Failed] {
        let persisted = undecided(resume_point.clone());
        for status in [
            WorkflowV2Status::Failed,
            WorkflowV2Status::Accepted,
            WorkflowV2Status::NeedsReview,
            WorkflowV2Status::Blocked,
        ] {
            assert_eq!(
                summary_disposition(
                    Some(&persisted),
                    WorkflowRunKind::AuthoredTaskWorkflow,
                    status
                )
                .unwrap_or_else(|error| panic!("{resume_point:?} -> {status:?}: {error}")),
                Disposition::Commit,
                "{resume_point:?} -> {status:?} is a first decision, not a changed identity"
            );
        }
    }
}

#[test]
fn an_unchanged_terminal_status_is_a_replay_not_a_second_commit() {
    for status in [WorkflowV2Status::Accepted, WorkflowV2Status::Failed] {
        assert_eq!(
            summary_disposition(
                Some(&decided(status)),
                WorkflowRunKind::AuthoredTaskWorkflow,
                status
            )
            .expect("unchanged status"),
            Disposition::Replay
        );
    }
    let paused = undecided(RunStatus::Paused);
    assert_eq!(
        run_status_disposition(
            Some(&paused),
            WorkflowRunKind::AuthoredTaskWorkflow,
            &RunStatus::Paused
        )
        .expect("unchanged status"),
        Disposition::Replay
    );
}

#[test]
fn a_different_run_kind_is_still_refused_as_corrupt() {
    let error = summary_disposition(
        Some(&decided(WorkflowV2Status::Failed)),
        WorkflowRunKind::FixedOrSavedScript,
        WorkflowV2Status::Failed,
    )
    .expect_err("state from another workflow must not be adopted");
    assert!(error.to_string().contains("identity changed"), "{error}");

    let error = run_status_disposition(
        Some(&undecided(RunStatus::Paused)),
        WorkflowRunKind::FixedOrSavedScript,
        &RunStatus::Paused,
    )
    .expect_err("state from another workflow must not be adopted");
    assert!(error.to_string().contains("identity changed"), "{error}");
}

/// The intent, pinned: a decided terminal status is replaceable exactly when
/// the run it describes is still resumable. Only a completing outcome ends the
/// run, so only a completing outcome may never be contradicted.
#[test]
fn a_decided_status_may_be_superseded_unless_it_completed_the_run() {
    for open in [
        WorkflowV2Status::Failed,
        WorkflowV2Status::Blocked,
        WorkflowV2Status::NeedsReview,
        WorkflowV2Status::Cancelled,
    ] {
        assert_eq!(
            summary_disposition(
                Some(&decided(open)),
                WorkflowRunKind::AuthoredTaskWorkflow,
                WorkflowV2Status::Accepted,
            )
            .unwrap_or_else(|error| panic!("{open:?} -> Accepted: {error}")),
            Disposition::Commit,
            "a run left resumable by {open:?} may record a later attempt's outcome"
        );
    }
    for completing in [WorkflowV2Status::Accepted, WorkflowV2Status::Noop] {
        let error = summary_disposition(
            Some(&decided(completing)),
            WorkflowRunKind::AuthoredTaskWorkflow,
            WorkflowV2Status::Failed,
        )
        .expect_err("a completed run cannot be contradicted");
        assert!(error.to_string().contains("completed"), "{error}");
        let error = run_status_disposition(
            Some(&decided(completing)),
            WorkflowRunKind::AuthoredTaskWorkflow,
            &RunStatus::Failed,
        )
        .expect_err("a completed run cannot be contradicted");
        assert!(error.to_string().contains("completed"), "{error}");
    }
}

#[test]
fn a_run_with_no_record_at_all_commits() {
    assert_eq!(
        summary_disposition(
            None,
            WorkflowRunKind::AuthoredTaskWorkflow,
            WorkflowV2Status::Failed
        )
        .expect("no record"),
        Disposition::Commit
    );
    assert_eq!(
        run_status_disposition(
            None,
            WorkflowRunKind::AuthoredTaskWorkflow,
            &RunStatus::Paused
        )
        .expect("no record"),
        Disposition::Commit
    );
}
