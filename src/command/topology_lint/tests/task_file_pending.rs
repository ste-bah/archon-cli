use super::*;

fn republish_acceptance_without_skeleton_provenance(root: &Path, task: &Path) {
    use archon_workflow::task_set_contract::{
        ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceLock, AcceptancePin,
        content_digest,
    };
    let tasks = task.parent().unwrap();
    let contract_path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let mut contract = std::fs::read(&contract_path).unwrap();
    contract.push(b'\n');
    std::fs::write(&contract_path, &contract).unwrap();
    let digest = content_digest(&contract);
    let lock_path = tasks.join(ACCEPTANCE_LOCK_FILE);
    let mut lock: AcceptanceLock =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    lock.digest.clone_from(&digest);
    std::fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(root, tasks);
    let mut pin: AcceptancePin =
        serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    pin.acceptance_digest = digest;
    pin.freeze_event_id = "acceptance-after-skeleton".into();
    pin.skeleton_digest = None;
    pin.skeleton_gate = None;
    std::fs::write(pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
}

#[test]
fn an_intact_skeleton_from_the_prior_acceptance_is_linted_as_a_draft_everywhere() {
    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let tasks = task.parent().unwrap();
    republish_acceptance_without_skeleton_provenance(temp.path(), &task);

    let source = LintSource::TaskFile(task.clone());
    let report = run_lint(temp.path(), &source).unwrap();
    assert!(
        report.contains("stale intact skeleton is linted as an unfrozen draft"),
        "{report}"
    );
    assert!(
        blocking_findings(temp.path(), &source).is_empty(),
        "{report}"
    );
    crate::command::topology_lint::preflight::task_file_freeze(temp.path(), &task)
        .expect("preflight accepts the same pending skeleton state");

    let set = crate::command::topology_lint::task_set::inspect(
        temp.path(),
        tasks,
        archon_core::config::GateMode::Observe,
    )
    .unwrap();
    assert!(
        set.report.contains("linted as an unfrozen draft"),
        "{}",
        set.report
    );
    assert!(set.blockers.is_empty(), "{:?}", set.blockers);
}

#[test]
fn a_pending_skeleton_that_no_longer_matches_its_lock_is_refused_everywhere() {
    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let tasks = task.parent().unwrap();
    republish_acceptance_without_skeleton_provenance(temp.path(), &task);
    std::fs::write(
        tasks.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
        b"tampered skeleton",
    )
    .unwrap();

    let source = LintSource::TaskFile(task.clone());
    let findings = blocking_findings(temp.path(), &source);
    assert!(
        findings
            .iter()
            .any(|finding| finding.contains("does not verify")),
        "{findings:?}"
    );
    let preflight = crate::command::topology_lint::preflight::task_file_freeze(temp.path(), &task)
        .unwrap_err()
        .to_string();
    assert!(preflight.contains("does not verify"), "{preflight}");
    let set = crate::command::topology_lint::task_set::inspect(
        temp.path(),
        tasks,
        archon_core::config::GateMode::Observe,
    )
    .err()
    .expect("task-set lint refuses a skeleton that differs from its lock")
    .to_string();
    assert!(set.contains("does not verify"), "{set}");
}
