use super::*;
use crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven as frozen_set;
use archon_workflow::task_set_contract::{AcceptanceCheck, TrustedCwd};

fn scope(
    set: &crate::command::workflow_task_set::republish::test_fixture::FrozenSet,
) -> AuthorScope {
    AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd)
        .expect("the task set's repository record is believed")
}

#[test]
fn candidate_selection_returns_none_found_without_a_matching_existing_path() {
    let set = frozen_set(&[("AC-X-001", "run", false)]);
    let mut scope = scope(&set);
    scope.task_universe = archon_workflow::task_universe::WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![],
    };
    assert_eq!(
        super::author::candidate_files(&scope, &set.contract().acceptance[0]),
        "none found"
    );
}

#[test]
fn prompt_includes_content_for_a_path_named_by_the_existing_check() {
    let set = frozen_set(&[("AC-X-001", "inspect src/decision.data", false)]);
    let mut frozen = set.contract().acceptance[0].clone();
    frozen.check = AcceptanceCheck::Command {
        command: "inspect src/decision.data".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    let path = set.project.path().join("src/decision.data");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "decision is correct\n").unwrap();
    let prompt = super::author::author_prompt(
        &scope(&set),
        &frozen,
        &["judge rejection: inspect src/decision.data".into()],
        1,
    );
    assert!(prompt.contains("decision is correct"), "{prompt}");
    assert!(
        prompt
            .contains("Answer once you have read the supplied named_files content for the deciding file, or inspected that file at its named path yourself."),
        "{prompt}"
    );
}

#[test]
fn prompt_uses_named_file_redaction_and_committed_content_rules() {
    let set = frozen_set(&[("AC-X-001", "inspect src/decision.data", false)]);
    let mut frozen = set.contract().acceptance[0].clone();
    frozen.check = AcceptanceCheck::Command {
        command: "inspect src/decision.data".into(),
        cwd: TrustedCwd::RepoRoot,
    };
    let path = scope(&set).repository_root.join("src/decision.data");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "uncommitted assertion\n").unwrap();
    let prompt = super::author::author_prompt(
        &scope(&set),
        &frozen,
        &["judge rejection: inspect src/decision.data".into()],
        1,
    );
    assert!(!prompt.contains("uncommitted assertion"), "{prompt}");
    assert!(prompt.contains("not_supplied"), "{prompt}");
}

#[test]
fn prompt_does_not_include_secret_values_from_named_content() {
    let set = frozen_set(&[("AC-X-001", "inspect src/private.data", false)]);
    let mut frozen = set.contract().acceptance[0].clone();
    frozen.check = AcceptanceCheck::Command {
        command: "inspect src/private.data".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    let path = set.project.path().join("src/private.data");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "credential = sk-abcdefghijklmnopqrst0000\n").unwrap();
    let prompt = super::author::author_prompt(
        &scope(&set),
        &frozen,
        &["judge rejection: inspect src/private.data".into()],
        1,
    );
    assert!(prompt.contains("***REDACTED***"), "{prompt}");
    assert!(!prompt.contains("sk-abcdefghijklmnopqrst0000"), "{prompt}");
}

#[test]
fn oversized_prefetch_keeps_output_contract_and_id_ahead_of_content() {
    let set = frozen_set(&[("AC-X-001", "run", false)]);
    let scope = scope(&set);
    let frozen = set.contract().acceptance[0].clone();
    let huge = serde_json::json!({"content": "x".repeat(200_000)});
    let prompt = super::author::author_prompt_with_prefetch(
        &scope,
        &frozen,
        &["finding".into()],
        1,
        &[huge],
    );
    let fitted_head = &prompt[..prompt.find("Files already identified").unwrap()];
    assert!(fitted_head.contains("Return one JSON object with id, criterion, check"));
    assert!(fitted_head.contains(&format!("Use the exact id {}", frozen.id)));
    assert!(
        fitted_head.contains("The host also runs the check on the tree before any implementation")
    );
    assert!(fitted_head.contains(
        "The check is a valid regression guard only when its pass or fail is decided by that test"
    ));
}
