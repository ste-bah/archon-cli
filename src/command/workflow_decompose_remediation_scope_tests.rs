use super::*;
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceCriterion, JudgeDecision, JudgeVerdict, TrustedCwd,
};

fn valid_task(task_id: &str, req: &str, file: &str) -> String {
    format!(
        "```yaml\ntask_id: {task_id}\ntitle: Body\ncomplexity: medium\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: [{req}]\nrequired_env_keys: []\nrequired_tools: [sh]\ndeliverable_contracts: []\n```\n\n## Files Expected to Change\n- `{file}` — output\n"
    )
}

#[test]
fn reauthor_scope_keeps_candidates_from_good_tasks_when_one_body_is_malformed() {
    let project = tempfile::tempdir().unwrap();
    let tasks = project.path().join("tasks/set");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::create_dir_all(project.path().join("src")).unwrap();
    std::fs::write(project.path().join("src/good.rs"), "pub fn good() {}\n").unwrap();
    std::fs::write(
        tasks.join("TASK-X-010.md"),
        valid_task("TASK-X-010", "REQ-X-010", "src/good.rs"),
    )
    .unwrap();
    std::fs::write(
        tasks.join("TASK-X-020.md"),
        "# generated decomposed PRD workflow",
    )
    .unwrap();

    let scope = resume_author_scope(project.path(), &tasks, &project.path().join("PRD.md"))
        .expect("malformed task files are omitted from advisory prompt context");
    assert_eq!(scope.task_universe.tasks.len(), 1);
    assert_eq!(scope.task_universe.tasks[0].canonical_task_id, "TASK-X-010");

    let frozen = AcceptanceCriterion {
        id: "AC-X-001".into(),
        criterion: "criterion".into(),
        check: AcceptanceCheck::Command {
            command: "true".into(),
            cwd: TrustedCwd::ProjectRoot,
        },
        gap_permitted: false,
        covers: vec!["REQ-X-010".into()],
        judgment: JudgeVerdict {
            verdict: JudgeDecision::Accepted,
            counterexample: String::new(),
            reason: String::new(),
            host_call_id: String::new(),
            sampling: None,
        },
    };
    assert!(
        crate::command::workflow_task_set::reauthor::author::candidate_files(&scope, &frozen)
            .contains("good.rs")
    );
}

#[test]
fn reauthor_scope_reports_an_unreadable_task_root_as_operational_error() {
    let project = tempfile::tempdir().unwrap();
    let tasks = project.path().join("missing/tasks");

    let error = resume_author_scope(project.path(), &tasks, &project.path().join("PRD.md"))
        .err()
        .expect("missing task root is operational failure");
    assert!(
        error.to_string().contains(&tasks.display().to_string()),
        "{error:#}"
    );
}
