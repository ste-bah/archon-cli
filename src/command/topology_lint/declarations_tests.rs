use std::fs;

use super::*;

/// A realistic task file: the fenced yaml block the parser requires, then the
/// markdown sections after it.
fn task(id: &str, tools: &str, env: &str, tail: &str) -> String {
    format!(
        "# {id} — Thing\n\n\
         ```yaml\n\
         task_id: {id}\n\
         title: Thing\n\
         complexity: medium\n\
         status: pending\n\
         depends_on: []\n\
         blocks: []\n\
         implements: []\n\
         required_env_keys: {env}\n\
         required_tools: {tools}\n\
         deliverable_contracts: []\n\
         ```\n\n{tail}\n"
    )
}

fn lint(files: &[String]) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    for (index, contents) in files.iter().enumerate() {
        let id = format!("TASK-X-{:03}", index + 1);
        fs::write(dir.path().join(format!("{id}.md")), contents).expect("write");
    }
    section(Some(dir.path()))
}

/// The live case: a task declaring no tools while running cargo. The host
/// builds its allowlist from that field, so the declaration is load-bearing.
#[test]
fn a_runner_used_but_not_declared_is_reported() {
    let out = lint(&[task(
        "TASK-X-001",
        "[]",
        "[]",
        "## Focused Tests\n\n- `cargo test -p thing`\n",
    )]);
    assert!(out.contains("`cargo`"), "{out}");
    assert!(out.contains("required_tools"), "{out}");
}

#[test]
fn a_declared_runner_is_not_reported() {
    let out = lint(&[task(
        "TASK-X-001",
        "[cargo]",
        "[]",
        "## Focused Tests\n\n- `cargo test -p thing`\n",
    )]);
    assert!(out.contains("every runner and environment key"), "{out}");
}

/// A backticked span that is prose about a CLI, not an invocation, must not be
/// reported — the same distinction the focused-test classifier draws.
#[test]
fn a_non_runner_first_token_is_not_a_missing_tool() {
    let out = lint(&[task(
        "TASK-X-001",
        "[]",
        "[]",
        "## Focused Tests\n\n- see `data list --json` output\n",
    )]);
    assert!(
        !out.contains("without declaring it in `required_tools`"),
        "a described CLI fragment is not an invocation: {out}"
    );
}

#[test]
fn an_env_key_referenced_but_not_declared_is_reported() {
    let out = lint(&[task(
        "TASK-X-001",
        "[bash]",
        "[]",
        "## Focused Tests\n\n- `bash -lc 'test -n \"$POLYGON_API_KEY\"'`\n",
    )]);
    assert!(out.contains("POLYGON_API_KEY"), "{out}");
    assert!(out.contains("required_env_keys"), "{out}");
}

/// Positional arguments and lower-case locals are not environment keys.
#[test]
fn positional_and_lowercase_shell_names_are_not_env_keys() {
    assert!(referenced_env_keys("echo $1 $2 $dir").is_empty());
    assert_eq!(
        referenced_env_keys("curl ${OPENBB_API_URL}/x"),
        ["OPENBB_API_URL".to_string()].into_iter().collect()
    );
}

/// A longer heading must still be read, or the section reports a clean task
/// because it located no commands at all — a false pass, the worst outcome.
#[test]
fn a_longer_focused_tests_heading_is_still_read() {
    let out = lint(&[task(
        "TASK-X-001",
        "[]",
        "[]",
        "## Focused Tests and Evidence\n\n- `cargo test -p thing`\n",
    )]);
    assert!(out.contains("`cargo`"), "{out}");
}

/// Only the Focused Tests section is read; a command quoted elsewhere in the
/// document is not a declaration of intent to run it.
#[test]
fn commands_outside_focused_tests_are_ignored() {
    let out = lint(&[task(
        "TASK-X-001",
        "[]",
        "[]",
        "## Notes\n\n- historically we ran `cargo bench`\n\n## Focused Tests\n\n- none yet\n",
    )]);
    assert!(
        !out.contains("without declaring it in `required_tools`"),
        "a command quoted outside Focused Tests is not a declaration of intent: {out}"
    );
}

#[test]
fn without_a_tasks_root_the_section_says_why_it_is_empty() {
    assert!(section(None).contains("only computed for --tasks"));
}

/// The worse defect, and the one this section was blind to: a task with no
/// runnable verifier at all. The old summary called it clean, because with
/// nothing parsed there was no undeclared runner to report.
///
/// A fenced block is NOT an example of this any more — those commands are read
/// now. Only genuinely command-free prose qualifies.
#[test]
fn a_task_with_no_runnable_focused_test_is_reported() {
    let out = lint(&[task(
        "TASK-X-001",
        "[cargo]",
        "[]",
        "## Focused Tests\n\n- verify the parser handles the migration\n- check the report reads well\n",
    )]);
    assert!(out.contains("NO runnable focused test"), "{out}");
    assert!(out.contains("TASK-X-001"), "{out}");
}

/// And it must not fire for a task that does declare one.
#[test]
fn a_task_with_a_runnable_focused_test_is_not_reported() {
    let out = lint(&[task(
        "TASK-X-001",
        "[cargo]",
        "[]",
        "## Focused Tests\n\n- `cargo test -p thing`\n",
    )]);
    assert!(!out.contains("NO runnable focused test"), "{out}");
}


/// Commands in a fenced block are read, so such a task is not command-free.
/// This is the pair to the parser change: lint and parser must agree, or the
/// lint reports a defect the engine does not see.
#[test]
fn a_fenced_block_of_commands_is_not_a_command_free_task() {
    let out = lint(&[task(
        "TASK-X-001",
        "[cargo]",
        "[]",
        "## Focused Tests\n\n```bash\ncargo test -p thing\n```\n",
    )]);
    assert!(!out.contains("NO runnable focused test"), "{out}");
}

/// `$PWD` and friends come from the shell, not from project configuration.
#[test]
fn shell_provided_variables_are_not_reported_as_undeclared() {
    assert!(referenced_env_keys("bash -lc 'cd $PWD && ls $HOME'").is_empty());
    assert_eq!(
        referenced_env_keys("bash -lc 'echo $POLYGON_API_KEY'"),
        ["POLYGON_API_KEY".to_string()].into_iter().collect()
    );
}
