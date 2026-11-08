use super::*;

#[test]
fn s4a_diagnostic_lock_fields_leave_verification_and_trace_verdicts_unchanged() {
    use crate::command::requirement_trace::{TraceOptions, run_trace};
    use archon_workflow::baseline_results::{BaselineCheckResult, BaselineOutcome, results_digest};
    use archon_workflow::task_set_contract::AcceptanceLock;
    use archon_workflow::task_skeleton::TaskSkeletonLock;
    use archon_workflow::v2::requirement_classes::RequirementClassification;
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    freeze_skeleton(&fixture);
    let trace_options = TraceOptions::new(fixture.prd.clone(), fixture.tasks.clone());
    let before = run_trace(&fixture.project, &trace_options).unwrap();
    for stage in [FrozenStage::Acceptance, FrozenStage::Skeleton] {
        verify_frozen_stage_with_recovery(
            &fixture.project,
            &fixture.prd,
            &fixture.tasks,
            stage,
            true,
        )
        .unwrap();
    }
    let path = fixture.tasks.join(ACCEPTANCE_LOCK_FILE);
    let mut lock: AcceptanceLock = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    lock.baseline_results.insert(
        "AC-X-001".into(),
        BaselineCheckResult {
            check_digest: "diagnostic".into(),
            outcome: BaselineOutcome::Passed,
        },
    );
    let record = RequirementClassification {
        schema_version: 1,
        baseline_commit: lock.baseline_commit.clone(),
        acceptance_digest: lock.digest.clone(),
        baseline_results_digest: results_digest(&lock.baseline_results),
        error: Some("diagnostic only".into()),
        entries: Vec::new(),
    };
    std::fs::write(path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
    let path = fixture.tasks.join(TASK_SKELETON_LOCK_FILE);
    let mut lock: TaskSkeletonLock =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    lock.classification = Some(record);
    std::fs::write(path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
    for stage in [FrozenStage::Acceptance, FrozenStage::Skeleton] {
        verify_frozen_stage_with_recovery(
            &fixture.project,
            &fixture.prd,
            &fixture.tasks,
            stage,
            true,
        )
        .unwrap();
    }
    let after = run_trace(&fixture.project, &trace_options).unwrap();
    assert_eq!(before, after);
}
