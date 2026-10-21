use std::path::Path;

use super::{Markers, named_inputs, wrap};

fn entry(command: &str) -> archon_workflow::task_set_contract::AcceptanceCriterion {
    serde_json::from_value(serde_json::json!({
        "id": "AC-TEST-001",
        "criterion": "the command checks its input",
        "check": {"kind": "command", "command": command, "cwd": "project_root"},
        "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": "test"}
    }))
    .unwrap()
}

fn run(dir: &Path, script: &str) -> i32 {
    super::test_helpers::run_with_host_mutation(dir, script)
        .status
        .code()
        .unwrap()
}

fn interpreter_on_path(dir: &Path, interpreter: &str) -> bool {
    run(dir, &format!("command -v {interpreter} >/dev/null 2>&1")) == 0
}

#[test]
fn inline_python_literal_read_survives_unrelated_shell_metacharacters_and_fails_when_moved() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("fixtures")).unwrap();
    std::fs::write(project.path().join("fixtures/input.txt"), "ready\n").unwrap();
    let command = "python3 -c 'p=\"fixtures/input.txt\"; t=open(p, encoding=\"utf-8\").read(); print(\"$ unrelated ` text\"); assert t'";
    let names = named_inputs(&entry(command), project.path(), project.path());
    assert_eq!(names, ["fixtures/input.txt"]);
    assert_eq!(run(project.path(), command), 0);
    assert_ne!(
        run(project.path(), &wrap(command, &names, &[], &Markers::new())),
        0
    );
}

#[test]
fn inline_program_printing_a_path_does_not_name_a_read_dependency() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("fixture.txt"), "present\n").unwrap();
    let command = "python3 -c 'print(\"fixture.txt\")'";
    let names = named_inputs(&entry(command), project.path(), project.path());
    assert_eq!(names, ["fixture.txt"]);
    assert_eq!(run(project.path(), command), 0);
    assert_eq!(
        run(project.path(), &wrap(command, &names, &[], &Markers::new())),
        0
    );
}

#[test]
fn inline_pathlib_read_is_a_candidate_and_fails_when_moved() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("fixtures")).unwrap();
    std::fs::write(project.path().join("fixtures/input.txt"), "ready\n").unwrap();
    let command = "python3 -c 'from pathlib import Path; text=Path(\"fixtures/input.txt\").read_text(); assert text'";
    let names = named_inputs(&entry(command), project.path(), project.path());
    assert_eq!(names, ["fixtures/input.txt"]);
    assert_eq!(run(project.path(), command), 0);
    assert_ne!(
        run(project.path(), &wrap(command, &names, &[], &Markers::new())),
        0
    );
}

#[test]
fn inline_program_and_heredoc_body_have_the_same_candidates() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("fixtures")).unwrap();
    std::fs::write(
        project.path().join("fixtures/input.json"),
        "{\"ready\": true}\n",
    )
    .unwrap();
    let program = "grep -q ready fixtures/input.json && echo done";
    for interpreter in ["sh", "dash", "bash", "zsh"] {
        let inline = format!("{interpreter} -c '{program}'");
        let heredoc = format!("{interpreter} <<'SCRIPT'\n{program}\nSCRIPT\n");
        assert_eq!(
            named_inputs(&entry(&inline), project.path(), project.path()),
            named_inputs(&entry(&heredoc), project.path(), project.path()),
            "{interpreter}"
        );
    }
}

#[test]
fn zsh_inline_program_finds_the_literal_check_input_and_fails_when_moved() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("fixtures")).unwrap();
    std::fs::write(project.path().join("fixtures/input.txt"), "ready\n").unwrap();
    let command = "zsh -c 'grep -q ready fixtures/input.txt && echo \"$HOME\"'";
    let names = named_inputs(&entry(command), project.path(), project.path());
    assert_eq!(names, ["fixtures/input.txt"]);
    if !interpreter_on_path(project.path(), "zsh") {
        eprintln!("skipping execution: zsh not on PATH");
        return;
    }
    assert_eq!(run(project.path(), command), 0);
    assert_ne!(
        run(project.path(), &wrap(command, &names, &[], &Markers::new())),
        0
    );
}

#[test]
fn dash_inline_program_finds_the_literal_check_input_and_fails_when_moved() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("fixtures")).unwrap();
    std::fs::write(project.path().join("fixtures/input.txt"), "ready\n").unwrap();
    let command = "dash -c 'grep -q ready fixtures/input.txt && echo \"$HOME\"'";
    let names = named_inputs(&entry(command), project.path(), project.path());
    assert_eq!(names, ["fixtures/input.txt"]);
    if !interpreter_on_path(project.path(), "dash") {
        eprintln!("skipping execution: dash not on PATH");
        return;
    }
    assert_eq!(run(project.path(), command), 0);
    assert_ne!(
        run(project.path(), &wrap(command, &names, &[], &Markers::new())),
        0
    );
}

#[test]
fn perl_multiple_inline_programs_find_a_literal_read_with_shell_metacharacters() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("fixtures")).unwrap();
    std::fs::write(project.path().join("fixtures/input.txt"), "ready\n").unwrap();
    let command = "perl -e '1;' -e 'open(my $f, \"<\", \"fixtures/input.txt\") or die; print while <$f>; print \"$ENV{HOME}\"'";
    let names = named_inputs(&entry(command), project.path(), project.path());
    assert_eq!(names, ["fixtures/input.txt"]);
    if !interpreter_on_path(project.path(), "perl") {
        eprintln!("skipping execution: perl not on PATH");
        return;
    }
    assert_eq!(run(project.path(), command), 0);
    assert_ne!(
        run(project.path(), &wrap(command, &names, &[], &Markers::new())),
        0
    );
}

#[test]
fn grep_pattern_argument_does_not_create_a_vacuous_candidate() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("fixtures")).unwrap();
    std::fs::write(project.path().join("fixtures/input.txt"), "x\n").unwrap();
    std::fs::write(project.path().join("x"), "unrelated pattern-named file\n").unwrap();
    let command = "grep -e 'x' fixtures/input.txt";
    let names = named_inputs(&entry(command), project.path(), project.path());
    assert!(
        names.contains(&"fixtures/input.txt".to_string()),
        "{names:?}"
    );
    assert_eq!(run(project.path(), command), 0);
    assert_eq!(
        run(
            project.path(),
            &wrap(command, &["x".to_string()], &[], &Markers::new())
        ),
        0,
        "the -e pattern word must not prove a read dependency"
    );
    assert_ne!(
        run(project.path(), &wrap(command, &names, &[], &Markers::new())),
        0
    );
}

#[test]
fn inline_json_read_using_os_path_join_fails_when_its_input_is_moved() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("fixtures")).unwrap();
    std::fs::write(
        project.path().join("fixtures/input.json"),
        "{\"ready\": true}\n",
    )
    .unwrap();
    let command = "python3 -c 'import json, os; value=json.load(open(os.path.join(\"fixtures\", \"input.json\"))); assert value[\"ready\"]'";
    let names = named_inputs(&entry(command), project.path(), project.path());
    assert!(!names.is_empty(), "no input candidate found");
    assert_eq!(run(project.path(), command), 0);
    assert_ne!(
        run(project.path(), &wrap(command, &names, &[], &Markers::new())),
        0
    );
}

#[test]
fn inline_dynamic_paths_and_unknown_cwd_flows_are_unresolved() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("fixtures")).unwrap();
    std::fs::write(project.path().join("fixtures/input.txt"), "ready\n").unwrap();
    let dynamic_program = "import os; p=\"fixtures/\" + os.getenv(\"NAME\"); open(p)";
    let inline = format!("python3 -c '{dynamic_program}'");
    let heredoc = format!("python3 - <<'PY'\n{dynamic_program}\nPY\n");
    let inline_names = named_inputs(&entry(&inline), project.path(), project.path());
    assert!(inline_names.is_empty(), "{inline_names:?}");
    assert_eq!(
        inline_names,
        named_inputs(&entry(&heredoc), project.path(), project.path())
    );
    for command in [
        "cd \"$UNKNOWN\" && python3 -c 'open(\"fixtures/input.txt\")'",
        "if cd elsewhere; then python3 -c 'open(\"fixtures/input.txt\")'; fi",
    ] {
        assert!(
            named_inputs(&entry(command), project.path(), project.path()).is_empty(),
            "{command}"
        );
    }
}
