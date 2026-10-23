//! O1: the skeleton freeze refuses, before anything is published, a
//! re-authored skeleton that drops a task the published skeleton froze; the
//! refusal is the skeleton author's finding.

use super::*;
use archon_workflow::task_set_contract::TASK_SKELETON_FILE;

fn skeleton(task: &str) -> String {
    format!(
        r#"{{"schema_version":1,"acceptance_digest":"untrusted-draft-link","tasks":[{{"task_id":"{task}","file_name":"{task}-body.md","depends_on":[],"blocks":[],"implements":["AC-X-001"],"deliverable_contracts":[]}}]}}"#
    )
}

#[test]
fn a_candidate_that_drops_a_frozen_task_is_refused_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, _) = seed_frozen_acceptance(&temp);
    let prd = temp.path().join("prds/PRD-X.md");
    std::fs::write(tasks.join(TASK_SKELETON_FILE), skeleton("TASK-X-010")).unwrap();
    freeze_skeleton(temp.path(), &tasks, &prd).expect("the first skeleton freezes");
    let pin_path = acceptance_pin_path(temp.path(), &tasks);
    let (pin, published) = (
        std::fs::read(&pin_path).unwrap(),
        std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap(),
    );

    let error = prepare_skeleton_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        skeleton("TASK-X-020").into_bytes(),
    )
    .expect_err("a skeleton that drops a frozen task never freezes");
    assert!(
        crate::command::workflow_task_set::CandidateRejected::caused(&error),
        "the skeleton author's finding: {error:#}"
    );
    assert!(
        format!("{error:#}").contains("drops frozen task TASK-X-010"),
        "{error:#}"
    );
    assert_eq!(std::fs::read(&pin_path).unwrap(), pin, "nothing published");
    assert_eq!(
        std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap(),
        published
    );

    // Keeping the frozen task and adding one is a re-author it may make.
    let kept = r#"{"schema_version":1,"acceptance_digest":"x","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010-body.md","depends_on":[],"blocks":[],"implements":["AC-X-001"],"deliverable_contracts":[]},{"task_id":"TASK-X-020","file_name":"TASK-X-020-body.md","depends_on":[],"blocks":[],"implements":["AC-X-001"],"deliverable_contracts":[]}]}"#;
    let kept = prepare_skeleton_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        kept.as_bytes().to_vec(),
    );
    assert!(
        kept.as_ref()
            .err()
            .is_none_or(|error| !format!("{error:#}").contains("drops frozen task")),
        "{:?}",
        kept.err()
    );
}
