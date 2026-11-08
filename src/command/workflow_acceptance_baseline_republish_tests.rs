use super::*;
use crate::command::workflow_task_set::executability::ExecutabilityProbe;
use archon_workflow::baseline_results::{BaselineOutcome, BaselineResults};
use archon_workflow::task_set_contract::AcceptanceLock;

#[test]
fn s4a_republish_keeps_unchanged_outcomes_and_drops_reauthored_checks() {
    let set = test_fixture::frozen_set_proven(&[
        ("AC-F-001", "test -f one", true),
        ("AC-F-002", "test -f two", true),
    ]);
    let path = set.tasks.join(ACCEPTANCE_LOCK_FILE);
    let mut lock: AcceptanceLock = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let commit = set.probe.baseline_commit();
    lock.baseline_commit = commit.clone();
    let original = set.contract();
    lock.baseline_results = BaselineResults::observed(
        commit,
        &[
            ("AC-F-001".into(), BaselineOutcome::Passed),
            ("AC-F-002".into(), BaselineOutcome::Failed),
        ]
        .into(),
        &original,
    )
    .outcomes;
    std::fs::write(&path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
    let ids = BTreeSet::from(["AC-F-002".into()]);
    let request = ReauthorRequest {
        project_root: set.project.path(),
        tasks_root: &set.tasks,
        prd_path: &set.prd,
        ids: &ids,
        gate: set.gate(),
        trigger: "test",
    };
    let verified = verify::verify(&request).unwrap();
    let mut repaired = original;
    repaired.acceptance[1].check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
        command: "test -s two".into(),
        cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    };
    publish_chain_owned(&request, verified, repaired, Vec::new(), None, &ids).unwrap();
    let after: AcceptanceLock = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        after.baseline_results,
        [("AC-F-001".into(), lock.baseline_results["AC-F-001"].clone())].into()
    );
    let skeleton: TaskSkeletonLock =
        serde_json::from_slice(&std::fs::read(set.tasks.join(TASK_SKELETON_LOCK_FILE)).unwrap())
            .unwrap();
    let classification = skeleton.classification.unwrap();
    assert_eq!(classification.baseline_commit, after.baseline_commit);
    assert_eq!(
        classification.baseline_results_digest,
        archon_workflow::baseline_results::results_digest(&after.baseline_results)
    );
}
