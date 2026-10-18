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
fn discovered_sibling_paths_are_not_mutation_candidates() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("fixture.txt"), "ready\n").unwrap();
    assert!(
        named_inputs(
            &entry("grep -q ready ../sibling/fixture.txt"),
            &project,
            &project
        )
        .is_empty()
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
    assert!(
        named_inputs(
            &entry("cargo test --manifest-path ../sibling/Cargo.toml --test tests/fixture.rs"),
            &project,
            &project
        )
        .is_empty()
    );
}

#[test]
fn a_bare_word_found_only_under_a_sibling_is_not_an_input() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("Cargo.toml"), "[package]\nname='sibling'\n").unwrap();
    std::fs::write(sibling.join("config"), "data\n").unwrap();
    assert!(
        named_inputs(
            &entry("cargo test --manifest-path ../sibling/Cargo.toml config"),
            &project,
            &project
        )
        .is_empty()
    );
}

#[test]
fn cwd_relative_path_stays_in_the_check_root_when_sibling_also_has_it() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(project.join("fixtures")).unwrap();
    std::fs::create_dir_all(sibling.join("fixtures")).unwrap();
    std::fs::write(project.join("fixtures/data.txt"), "cwd\n").unwrap();
    std::fs::write(sibling.join("fixtures/data.txt"), "sibling\n").unwrap();
    std::fs::write(sibling.join("Cargo.toml"), "[package]\nname='sibling'\n").unwrap();
    assert_eq!(
        named_inputs(
            &entry("cargo test --manifest-path ../sibling/Cargo.toml fixtures/data.txt"),
            &project,
            &project
        ),
        ["fixtures/data.txt"]
    );
}

#[test]
fn a_path_in_a_script_body_is_not_rebased_by_another_commands_manifest() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(sibling.join("fixtures")).unwrap();
    std::fs::write(sibling.join("Cargo.toml"), "[package]\nname='sibling'\n").unwrap();
    std::fs::write(sibling.join("fixtures/data.json"), "{}\n").unwrap();
    let command = "python3 - <<'PY'\nopen('fixtures/data.json')\nPY\ncargo test --manifest-path ../sibling/Cargo.toml";
    assert_eq!(
        named_inputs(&entry(command), &project, &project),
        ["fixtures/data.json"]
    );
}

#[test]
fn explicit_sibling_path_with_case_mismatch_is_not_an_input() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("LICENSE"), "terms\n").unwrap();
    assert!(
        named_inputs(
            &entry("grep -q terms ../sibling/license"),
            &project,
            &project
        )
        .is_empty()
    );
}

#[test]
fn a_case_mismatched_bare_word_is_not_mapped_to_a_sibling_file() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("Cargo.toml"), "[package]\nname='sibling'\n").unwrap();
    std::fs::write(sibling.join("LICENSE"), "terms\n").unwrap();
    assert!(
        named_inputs(
            &entry("cargo test --manifest-path ../sibling/Cargo.toml license"),
            &project,
            &project
        )
        .is_empty()
    );
}

#[test]
fn later_commands_resolve_paths_from_the_effective_sibling_directory() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("archon-cli");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(sibling.join("crates/x/tests")).unwrap();
    std::fs::write(sibling.join("Cargo.toml"), "[workspace]\n").unwrap();
    std::fs::write(sibling.join("crates/x/tests/y.rs"), "#[test] fn y() {}\n").unwrap();
    assert!(
        named_inputs(
            &entry(
                "cd ../archon-cli && cargo test --manifest-path Cargo.toml && python3 -c 'open(\"crates/x/tests/y.rs\")'"
            ),
            &project,
            &project,
        )
        .is_empty()
    );
}

#[test]
fn subshell_directory_changes_do_not_leak_to_later_commands() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(project.join("fixtures")).unwrap();
    std::fs::create_dir_all(sibling.join("fixtures")).unwrap();
    std::fs::write(sibling.join("fixtures/data.txt"), "sibling\n").unwrap();
    std::fs::write(project.join("fixtures/data.txt"), "project\n").unwrap();
    assert_eq!(
        named_inputs(
            &entry("( cd ../sibling && true ) && grep -q project fixtures/data.txt"),
            &project,
            &project,
        ),
        ["fixtures/data.txt"]
    );
}

#[test]
fn pushd_popd_and_cd_dash_restore_the_effective_directory() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("first")).unwrap();
    std::fs::create_dir_all(project.path().join("second")).unwrap();
    std::fs::write(project.path().join("data.txt"), "root\n").unwrap();
    std::fs::write(project.path().join("first/data.txt"), "first\n").unwrap();
    std::fs::write(project.path().join("second/data.txt"), "second\n").unwrap();
    assert_eq!(
        named_inputs(
            &entry(
                "pushd first && grep -q first data.txt && popd && cd second && cd - && grep -q root data.txt"
            ),
            project.path(),
            project.path(),
        ),
        ["first/data.txt", "data.txt"]
    );
}

#[test]
fn unknown_cd_target_makes_later_relative_candidates_unresolved() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("fixtures")).unwrap();
    std::fs::write(project.path().join("fixtures/data.txt"), "project\n").unwrap();
    assert!(
        named_inputs(
            &entry("cd \"$CHECK_DIR\" && grep -q data fixtures/data.txt"),
            project.path(),
            project.path(),
        )
        .is_empty()
    );
}

#[test]
fn shell_comment_separators_and_paths_are_ignored() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("fixtures")).unwrap();
    std::fs::write(project.path().join("fixtures/data.txt"), "x\n").unwrap();
    assert_eq!(
        named_inputs(
            &entry("grep x fixtures/data.txt # ignored.txt; cd nowhere"),
            project.path(),
            project.path()
        ),
        ["fixtures/data.txt"]
    );
}

#[test]
fn cd_in_if_condition_makes_branch_directory_unresolved() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(sibling.join("fixtures")).unwrap();
    std::fs::write(sibling.join("Cargo.toml"), "[package]\nname='sibling'\n").unwrap();
    std::fs::write(sibling.join("fixtures/data.txt"), "x\n").unwrap();
    assert!(
        named_inputs(
            &entry("if cd ../sibling; then grep x fixtures/data.txt; fi"),
            &project,
            &project
        )
        .is_empty()
    );
}

#[test]
fn if_else_branches_start_at_the_if_directory_and_divergent_results_are_unknown() {
    let project = tempfile::tempdir().unwrap();
    for directory in ["a", "b"] {
        std::fs::create_dir(project.path().join(directory)).unwrap();
        std::fs::write(project.path().join(directory).join("data"), "x\n").unwrap();
    }
    std::fs::write(project.path().join("data"), "x\n").unwrap();
    assert_eq!(
        named_inputs(
            &entry(
                "if test -e flag; then cd a; grep x data; else cd b; grep x data; fi; grep x data"
            ),
            project.path(),
            project.path(),
        ),
        ["a/data", "b/data"]
    );
}

#[test]
fn loop_directory_changes_make_post_loop_relative_candidates_unresolved() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("a")).unwrap();
    std::fs::write(project.path().join("a/data"), "x\n").unwrap();
    std::fs::write(project.path().join("data"), "x\n").unwrap();
    assert!(
        !named_inputs(
            &entry("while test -e flag; do cd a; grep x data; done; grep x data"),
            project.path(),
            project.path(),
        )
        .contains(&"data".to_string())
    );
}

#[test]
fn short_circuit_directory_changes_merge_only_after_the_list() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("a")).unwrap();
    std::fs::create_dir(project.path().join("b")).unwrap();
    for path in ["data", "a/data", "b/data"] {
        std::fs::write(project.path().join(path), "x\n").unwrap();
    }

    let inputs = named_inputs(
        &entry("cd a && grep x data; grep x data"),
        project.path(),
        project.path(),
    );
    assert_eq!(inputs, ["a/data"]);

    assert!(
        named_inputs(
            &entry("cd a || cd b; grep x data"),
            project.path(),
            project.path(),
        )
        .is_empty()
    );
}

#[test]
fn failing_cd_short_circuits_and_set_e_keep_the_continuing_directory_known() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("a")).unwrap();
    std::fs::write(project.path().join("a/data"), "x\n").unwrap();

    for command in [
        "cd a || exit 1; grep x data",
        "cd a || { echo retrying; exit 1; }; grep x data",
        "cd a || return; grep x data",
        "set -e; cd a; grep x data",
    ] {
        assert_eq!(
            named_inputs(&entry(command), project.path(), project.path()),
            ["a/data"],
            "{command}"
        );
    }
}

#[test]
fn multiline_quoted_hash_line_does_not_drop_following_command() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("real_input"), "x\n").unwrap();
    assert_eq!(
        named_inputs(
            &entry("printf \"quoted text\n# still quoted\"; cat real_input"),
            project.path(),
            project.path(),
        ),
        ["real_input"]
    );
}

#[test]
fn env_and_git_directory_options_scope_candidate_resolution() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(sibling.join("fixtures")).unwrap();
    std::fs::write(sibling.join("fixtures/data.txt"), "x\n").unwrap();
    for command in [
        "env -C ../sibling grep x fixtures/data.txt",
        "git -C ../sibling grep x fixtures/data.txt",
    ] {
        assert!(named_inputs(&entry(command), &project, &project).is_empty());
    }
}

#[test]
fn literal_path_variables_are_substituted_but_unknown_variables_stay_unresolved() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(sibling.join("fixtures")).unwrap();
    std::fs::write(sibling.join("Cargo.toml"), "[package]\nname='sibling'\n").unwrap();
    std::fs::write(sibling.join("fixtures/data.txt"), "x\n").unwrap();
    assert!(
        named_inputs(
            &entry("ROOT=../sibling; grep x \"$ROOT/fixtures/data.txt\""),
            &project,
            &project
        )
        .is_empty()
    );
    std::fs::create_dir_all(project.join("UNKNOWN/fixtures")).unwrap();
    std::fs::write(project.join("UNKNOWN/fixtures/data.txt"), "x\n").unwrap();
    assert!(
        named_inputs(
            &entry("grep x \"$UNKNOWN/fixtures/data.txt\""),
            &project,
            &project
        )
        .is_empty()
    );
}

#[test]
fn interpreter_strings_and_heredocs_use_the_directory_after_cd() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(sibling.join("fixtures")).unwrap();
    std::fs::write(sibling.join("fixtures/data.json"), "{}\n").unwrap();
    let python = entry("cd ../sibling && python3 -c 'open(\"fixtures/data.json\")'");
    assert!(named_inputs(&python, &project, &project).is_empty());
    let heredoc = entry("cd ../sibling && python3 - <<'PY'\nopen('fixtures/data.json')\nPY\n");
    assert!(named_inputs(&heredoc, &project, &project).is_empty());
}

#[test]
fn heredoc_lexing_keeps_the_rest_of_the_command_line() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("real_input"), "x\n").unwrap();

    for command in [
        "python3 - <<'PY'; grep -q x real_input\nprint('script_input')\nPY\n",
        "python3 - <<A <<B; grep -q x real_input\nprint('first_body_input')\nA\nprint('second_body_input')\nB\n",
        "cat <<EOF | grep x real_input\nbody words\nEOF\n",
    ] {
        assert_eq!(
            named_inputs(&entry(command), project.path(), project.path()),
            ["real_input"]
        );
    }
}

#[test]
fn heredoc_bodies_are_not_lexed_as_shell_commands() {
    let (commands, bodies) = super::paths::lex(
        "python3 - <<'PY'; grep -q x real_input\ncat body_input | touch another_input\nPY\n",
    );

    assert!(
        commands
            .iter()
            .any(|words| words == &["grep", "-q", "x", "real_input"])
    );
    assert!(
        !commands
            .iter()
            .any(|words| words.iter().any(|word| word == "body_input"))
    );
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0].1, "cat body_input | touch another_input\n");
}

#[cfg(unix)]
#[path = "workflow_acceptance_executability_mutation_restoration_tests.rs"]
mod restoration_tests;
