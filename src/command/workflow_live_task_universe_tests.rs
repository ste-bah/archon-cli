use super::*;

#[test]
fn universe_comes_from_task_files_not_reducer_items() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\ntask_id: TASK-TDL-001\ndepends_on: []\n",
    )
    .expect("task 1");
    fs::write(
        temp.path().join("TASK-TDL-010-dependent.md"),
        "# Dependent\n\ntask_id: TASK-TDL-010\ndepends_on: ['TASK-TDL-001']\n",
    )
    .expect("task 10");

    let universe = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {}",
        temp.path().display()
    ))
    .expect("extract")
    .expect("universe");

    assert_eq!(
        universe.canonical_ids(),
        vec!["TASK-TDL-001".to_string(), "TASK-TDL-010".to_string()]
    );
    assert_eq!(
        universe.tasks[1].dependency_ids,
        vec!["TASK-TDL-001".to_string()]
    );
}

#[test]
fn prd_task_references_must_have_matching_task_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let prd = temp.path().join("PRD.md");
    fs::write(&prd, "Acceptance references TASK-TDL-140.\n").expect("prd");
    let tasks = temp.path().join("tasks");
    fs::create_dir_all(&tasks).expect("tasks");
    fs::write(
        tasks.join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\ntask_id: TASK-TDL-001\ndepends_on: []\n",
    )
    .expect("task");

    let err = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {} and tasks at {}",
        prd.display(),
        tasks.display()
    ))
    .expect_err("unbacked PRD task reference must fail");

    assert!(err.to_string().contains("references TASK-TDL-140"));
}

#[test]
fn missing_authoritative_task_evidence_fails_for_decomposed_prd() {
    let err = extract_task_universe_for_generated_run(
        "Implement the decomposed PRD at /no/such/tasks/PRD-MISSING",
    )
    .expect_err("missing local evidence must fail");

    assert!(
        err.to_string()
            .contains("requires local TASK-*.md evidence")
    );
}

#[test]
fn invalid_task_id_is_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\ntask_id: TASK-TDL-1\ndepends_on: []\n",
    )
    .expect("task");

    let err = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {}",
        temp.path().display()
    ))
    .expect_err("invalid task id must fail");

    assert!(err.to_string().contains("invalid task_id"));
}

#[test]
fn dependency_cycles_are_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("TASK-TDL-001-foundation.md"),
        "# Foundation\n\ntask_id: TASK-TDL-001\ndepends_on: [TASK-TDL-010]\n",
    )
    .expect("task 1");
    fs::write(
        temp.path().join("TASK-TDL-010-dependent.md"),
        "# Dependent\n\ntask_id: TASK-TDL-010\ndepends_on: [TASK-TDL-001]\n",
    )
    .expect("task 10");

    let err = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {}",
        temp.path().display()
    ))
    .expect_err("cycle must fail");

    assert!(err.to_string().contains("dependency cycle"));
}
