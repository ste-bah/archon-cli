use super::*;
use archon_workflow::task_set_contract::{TASK_SKELETON_FILE, content_digest};
use archon_workflow::task_skeleton::TaskSkeletonLock;

fn snapshot(body: &str) -> FrozenChainSnapshot {
    FrozenChainSnapshot {
        acceptance: true,
        skeleton: true,
        subjects: vec![archon_workflow::HostCommandSubject {
            task_id: "TASK-1".into(),
            file_name: "TASK-1.md".into(),
        }],
        bodies: vec!["TASK-1.md".into()],
        body_digests: vec![("TASK-1.md".into(), content_digest(body.as_bytes()))],
        preserved_digests: vec![],
    }
}

fn skeleton(acceptance_digest: &str, subject: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "acceptance_digest": acceptance_digest,
        "tasks": [{"task_id": subject, "file_name": format!("{subject}.md")}]
    }))
    .unwrap()
}

fn check(
    old_subject: &str,
    new_subject: &str,
    old_body: &str,
    new_body: &str,
) -> Result<(), String> {
    let old_skeleton = skeleton("old-contract", old_subject);
    let new_skeleton = skeleton("new-contract", new_subject);
    let mut old = snapshot(old_body);
    let mut new = snapshot(new_body);
    for (snapshot, bytes) in [(&mut old, &old_skeleton), (&mut new, &new_skeleton)] {
        snapshot
            .preserved_digests
            .push((TASK_SKELETON_FILE.into(), content_digest(bytes)));
    }
    let old_bodies = [("TASK-1.md".into(), old_body.as_bytes().to_vec())]
        .into_iter()
        .collect();
    let new_bodies = [("TASK-1.md".into(), new_body.as_bytes().to_vec())]
        .into_iter()
        .collect();
    compare_repaired_chain(
        &old,
        &new,
        Some(&old_skeleton),
        Some(&new_skeleton),
        "new-contract",
        &old_bodies,
        &new_bodies,
    )
}

#[test]
fn successful_repair_allows_republished_derived_lock_fields() {
    assert!(check("TASK-1", "TASK-1", "body", "body").is_ok());

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().canonicalize().unwrap();
    let prd = project.join("prds/PRD-X.md");
    let tasks = project.join("tasks/PRD-X");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        &prd,
        "# PRD X\n\n## Requirements\n\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | Prove the fixture. |\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-X-001 | The fixture is proven. |\n",
    )
    .unwrap();
    use super::super::workflow_decompose_frozen_chain::test_support as support;
    let acceptance_digest = support::freeze_acceptance(&project, &prd, &tasks);
    support::freeze_skeleton(&project, &tasks, &acceptance_digest, &["TASK-X-001"]);
    let lock_path = tasks.join(archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE);
    let mut lock: TaskSkeletonLock =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    lock.gate.evaluated_at = "2026-08-28T00:00:00Z".into();
    std::fs::write(&lock_path, serde_json::to_vec(&lock).unwrap()).unwrap();
    let pin_path = super::super::workflow_task_set::acceptance_pin_path(&project, &tasks);
    let mut pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    pin.skeleton_gate = Some(lock.gate);
    std::fs::write(pin_path, serde_json::to_vec(&pin).unwrap()).unwrap();

    let verified = frozen_chain_snapshot(&project, &prd, &tasks).unwrap();
    assert!(verified.skeleton);
    let skeleton_bytes = std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap();
    let contract_bytes =
        std::fs::read(tasks.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE))
            .unwrap();
    let lock: TaskSkeletonLock =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    assert_eq!(lock.digest, content_digest(&skeleton_bytes));
    assert_eq!(lock.acceptance_digest, content_digest(&contract_bytes));
}

#[test]
fn changed_skeleton_subject_is_rejected() {
    let error = check("TASK-1", "TASK-2", "body", "body").unwrap_err();
    assert!(error.contains("task-skeleton.json.tasks"), "{error}");
    assert!(
        error.contains("old digest") && error.contains("new digest"),
        "{error}"
    );
}

#[test]
fn changed_task_body_is_rejected() {
    let error = check("TASK-1", "TASK-1", "old body", "new body").unwrap_err();
    assert!(error.contains("TASK-1.md"), "{error}");
    assert!(
        error.contains("old digest") && error.contains("new digest"),
        "{error}"
    );
}

#[test]
fn acceptance_only_repair_skips_skeleton_and_body_comparisons() {
    let mut launched = snapshot("body");
    launched.skeleton = false;
    launched.subjects.clear();
    launched.bodies.clear();
    launched.body_digests.clear();
    let mut republished = launched.clone();

    assert!(
        compare_repaired_chain(
            &launched,
            &republished,
            None,
            None,
            "new-contract",
            &Default::default(),
            &Default::default(),
        )
        .is_ok()
    );

    republished.skeleton = true;
    let error = compare_repaired_chain(
        &launched,
        &republished,
        None,
        None,
        "new-contract",
        &Default::default(),
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("skeleton"), "{error}");
}

#[test]
fn frozen_chain_verifier_rejects_lock_digest_that_does_not_match_skeleton() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().canonicalize().unwrap();
    let prd = project.join("prds/PRD-X.md");
    let tasks = project.join("tasks/PRD-X");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        &prd,
        "# PRD X\n\n## Requirements\n\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | Prove the fixture. |\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-X-001 | The fixture is proven. |\n",
    )
    .unwrap();
    use super::super::workflow_decompose_frozen_chain::test_support as support;
    let acceptance_digest = support::freeze_acceptance(&project, &prd, &tasks);
    support::freeze_skeleton(&project, &tasks, &acceptance_digest, &["TASK-X-001"]);
    let lock_path = tasks.join(archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE);
    let mut lock: TaskSkeletonLock =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    lock.digest = "wrong-digest".into();
    std::fs::write(&lock_path, serde_json::to_vec(&lock).unwrap()).unwrap();

    let error = frozen_chain_snapshot(&project, &prd, &tasks).unwrap_err();
    assert!(format!("{error:#}").contains("digest"), "{error:#}");
}

#[test]
fn missing_skeleton_lock_keeps_only_acceptance_frozen() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().canonicalize().unwrap();
    let prd = project.join("prds/PRD-X.md");
    let tasks = project.join("tasks/PRD-X");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        &prd,
        "# PRD X\n\n## Requirements\n\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | Prove the fixture. |\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-X-001 | The fixture is proven. |\n",
    )
    .unwrap();
    use super::super::workflow_decompose_frozen_chain::test_support as support;
    let acceptance_digest = support::freeze_acceptance(&project, &prd, &tasks);
    support::freeze_skeleton(&project, &tasks, &acceptance_digest, &["TASK-X-001"]);
    std::fs::remove_file(tasks.join(archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE))
        .unwrap();

    let snapshot = frozen_chain_snapshot(&project, &prd, &tasks).unwrap();
    assert!(snapshot.acceptance);
    assert!(!snapshot.skeleton);
}
