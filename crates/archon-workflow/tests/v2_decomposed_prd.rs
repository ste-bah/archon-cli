use archon_workflow::{WorkflowV2PrdIntake, WorkflowV2PrdIntakeError, WorkflowV2TaskFileStatus};

#[test]
fn decomposed_fixture_creates_stable_task_records() {
    let fixture = DecomposedFixture::new();
    let intake =
        WorkflowV2PrdIntake::discover(&fixture.prd_path, &fixture.task_dir).expect("intake");

    assert_eq!(
        intake
            .task_records
            .iter()
            .map(|record| record.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["T001", "T010", "T020"]
    );
    assert_eq!(intake.task_records[1].depends_on, vec!["T001"]);
    assert_eq!(
        intake.task_records[0].candidate_target_files,
        vec!["src/lib.rs"]
    );
    assert!(
        intake.task_records[2]
            .acceptance_criteria
            .iter()
            .any(|criterion| criterion.contains("focused test passes"))
    );
}

#[test]
fn missing_task_dir_fails_clearly() {
    let fixture = DecomposedFixture::new();
    let err = WorkflowV2PrdIntake::discover(&fixture.prd_path, fixture.root.path().join("missing"))
        .expect_err("missing dir");

    assert!(matches!(err, WorkflowV2PrdIntakeError::MissingTaskDir(_)));
}

#[test]
fn task_ordering_is_preserved_from_readme() {
    let fixture = DecomposedFixture::new();
    let intake =
        WorkflowV2PrdIntake::discover(&fixture.prd_path, &fixture.task_dir).expect("intake");

    assert_eq!(intake.task_records[0].title, "Bootstrap boundary");
    assert_eq!(intake.task_records[1].title, "Implement parser");
    assert_eq!(intake.task_records[2].title, "Verify parser");
}

#[test]
fn hard_rules_are_preserved_from_prd_readme_context_and_task() {
    let fixture = DecomposedFixture::new();
    let intake =
        WorkflowV2PrdIntake::discover(&fixture.prd_path, &fixture.task_dir).expect("intake");
    let rules = &intake.task_records[1].hard_rules;

    assert!(
        rules
            .iter()
            .any(|rule| rule.contains("No broad test suite"))
    );
    assert!(rules.iter().any(|rule| rule.contains("No fake claims")));
    assert!(
        rules
            .iter()
            .any(|rule| rule.contains("Keep changed files small"))
    );
    assert!(
        rules
            .iter()
            .any(|rule| rule.contains("Parser must be generic"))
    );
}

#[test]
fn no_task_file_is_silently_skipped_when_targets_are_absent() {
    let fixture = DecomposedFixture::new();
    let intake =
        WorkflowV2PrdIntake::discover(&fixture.prd_path, &fixture.task_dir).expect("intake");

    let verify = intake
        .task_records
        .iter()
        .find(|record| record.task_id == "T020")
        .expect("T020 present");
    assert!(verify.candidate_target_files.is_empty());
    assert_eq!(
        verify.status_from_task_file,
        WorkflowV2TaskFileStatus::Blocked
    );
}

#[test]
fn mixed_case_metadata_and_repeated_sections_are_preserved() {
    let fixture = DecomposedFixture::new();
    std::fs::write(
        fixture.task_dir.join("TASK-GEN-030.md"),
        r#"# Case insensitive metadata
Task ID: task-gen-030
Depends-On: [task-gen-020]
Status: Done

Target-Files: src/case.rs

## Acceptance Criteria
- first criterion

## Acceptance Criteria
- second criterion

### Constraints
- repeated section rule
"#,
    )
    .expect("task30");
    std::fs::write(
        fixture.task_dir.join("README.md"),
        "# Index\n\n1. [t030](task-gen-030.md)\n2. [T001](TASK-GEN-001.md)\n3. [T010](TASK-GEN-010.md)\n4. [T020](TASK-GEN-020.md)\n",
    )
    .expect("readme");

    let intake =
        WorkflowV2PrdIntake::discover(&fixture.prd_path, &fixture.task_dir).expect("intake");
    let case_task = &intake.task_records[0];

    assert_eq!(case_task.task_id, "T030");
    assert_eq!(case_task.depends_on, vec!["T020"]);
    assert_eq!(
        case_task.status_from_task_file,
        WorkflowV2TaskFileStatus::Done
    );
    assert_eq!(case_task.candidate_target_files, vec!["src/case.rs"]);
    assert!(
        case_task
            .acceptance_criteria
            .iter()
            .any(|criterion| criterion.contains("first criterion"))
    );
    assert!(
        case_task
            .acceptance_criteria
            .iter()
            .any(|criterion| criterion.contains("second criterion"))
    );
    assert!(
        case_task
            .hard_rules
            .iter()
            .any(|rule| rule.contains("repeated section rule"))
    );
}

#[test]
fn duplicate_canonical_task_ids_fail_instead_of_merging_work() {
    let fixture = DecomposedFixture::new();
    std::fs::write(
        fixture.task_dir.join("TASK-GEN-DUPLICATE.md"),
        r#"# Duplicate id
task_id: T010
status: ready
"#,
    )
    .expect("duplicate");

    let err =
        WorkflowV2PrdIntake::discover(&fixture.prd_path, &fixture.task_dir).expect_err("duplicate");

    assert!(matches!(
        err,
        WorkflowV2PrdIntakeError::DuplicateTaskId { task_id, .. } if task_id == "T010"
    ));
}

struct DecomposedFixture {
    root: tempfile::TempDir,
    prd_path: std::path::PathBuf,
    task_dir: std::path::PathBuf,
}

impl DecomposedFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let prd_dir = root.path().join("prds");
        let task_dir = root.path().join("tasks/PRD-GENERIC-001");
        let context_dir = task_dir.join("context");
        std::fs::create_dir_all(&prd_dir).expect("prd dir");
        std::fs::create_dir_all(&context_dir).expect("context dir");
        let prd_path = prd_dir.join("PRD-GENERIC-001.md");

        std::fs::write(
            &prd_path,
            "# Generic PRD\n\n## Hard Rules\n- No broad test suite\n",
        )
        .expect("prd");
        std::fs::write(
            task_dir.join("README.md"),
            "# Index\n\n## Hard Rules\n- No fake claims\n\n1. [T001](TASK-GEN-001.md)\n2. [T010](TASK-GEN-010.md)\n3. [T020](TASK-GEN-020.md)\n",
        )
        .expect("readme");
        std::fs::write(
            context_dir.join("activeContext.md"),
            "## Constraints\n- Keep changed files small\n",
        )
        .expect("context");
        std::fs::write(
            task_dir.join("TASK-GEN-001.md"),
            r#"# Bootstrap boundary
task_id: TASK-GEN-001
status: ready

## Files Expected to Change
- `src/lib.rs`

## Acceptance Criteria
- boundary exists
"#,
        )
        .expect("task1");
        std::fs::write(
            task_dir.join("TASK-GEN-010.md"),
            r#"# Implement parser
task_id: TASK-GEN-010
depends_on: [TASK-GEN-001]
status: in_progress

target_files: [src/parser.rs]

## Hard Rules
- Parser must be generic

## Definition of Done
- parser handles markdown tasks
"#,
        )
        .expect("task10");
        std::fs::write(
            task_dir.join("TASK-GEN-020.md"),
            r#"# Verify parser
task_id: TASK-GEN-020
depends_on: [T010]
status: blocked

## Acceptance Criteria
- focused test passes
"#,
        )
        .expect("task20");

        Self {
            root,
            prd_path,
            task_dir,
        }
    }
}
