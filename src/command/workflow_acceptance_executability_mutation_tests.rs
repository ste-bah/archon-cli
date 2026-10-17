//! Focused real-filesystem tests for candidate extraction.

use super::*;

fn entry(command: &str) -> AcceptanceCriterion {
    serde_json::from_value(serde_json::json!({
        "id": "AC-TEST-001",
        "criterion": "the command checks its input",
        "check": {"kind": "command", "command": command, "cwd": "project_root"},
        "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": "test"}
    }))
    .unwrap()
}

#[test]
fn bare_words_need_to_exist_and_message_paths_are_ignored() {
    let project = tempfile::tempdir().unwrap();
    assert!(named_inputs(&entry("test -e tests"), project.path(), project.path()).is_empty());
    std::fs::create_dir(project.path().join("tests")).unwrap();
    assert_eq!(
        named_inputs(&entry("test -e tests"), project.path(), project.path()),
        ["tests"]
    );
    std::fs::write(project.path().join("diagnostic.txt"), "bad").unwrap();
    assert!(
        named_inputs(
            &entry("echo diagnostic.txt"),
            project.path(),
            project.path()
        )
        .is_empty()
    );
}

#[test]
fn generated_or_overwritten_paths_are_not_mutation_inputs() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("fixture.txt"), "old\n").unwrap();
    let command = "printf 'ready\\n' > fixture.txt && grep -q ready fixture.txt";
    assert!(named_inputs(&entry(command), project.path(), project.path()).is_empty());
}

#[test]
fn discovered_sibling_paths_remain_relative_to_the_sibling_tree() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("fixture.txt"), "ready\n").unwrap();
    assert_eq!(
        named_inputs(
            &entry("grep -q ready ../sibling/fixture.txt"),
            &project,
            &project
        ),
        ["../sibling/fixture.txt"]
    );
}

#[test]
fn manifest_relative_paths_are_resolved_inside_the_sibling_root() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(sibling.join("tests")).unwrap();
    std::fs::write(sibling.join("Cargo.toml"), "[package]\nname='sibling'\n").unwrap();
    std::fs::write(
        sibling.join("tests/fixture.rs"),
        "#[test] fn fixture() {}\n",
    )
    .unwrap();
    assert_eq!(
        named_inputs(
            &entry("cargo test --manifest-path ../sibling/Cargo.toml --test tests/fixture.rs"),
            &project,
            &project
        ),
        ["../sibling/tests/fixture.rs"]
    );
}

#[cfg(unix)]
#[path = "workflow_acceptance_executability_mutation_restoration_tests.rs"]
mod restoration_tests;
