//! Issue 554: a check text the executing shell rejects, and a run whose own
//! shell reported a syntax error, are never a pass.

use super::*;
use crate::acceptance_check_crash::{CheckRunClass, classify_check_run};
use crate::acceptance_scratch::CheckResult;
use std::io::Write;

/// The R11 shape: `trap ... EXIT` runs first, then a `{ ... fi` block that
/// no POSIX shell parses. Under bash's POSIX mode this exits 0.
const TRAP_THEN_PARSE_ERROR: &str = "set -eu\nlog=$(mktemp)\ntrap 'rm -f \"$log\"' EXIT\nf() {\n grep -q x /dev/null || {\n  echo b\n fi\n}\n";

fn result(exit: Option<i32>, stderr: &str) -> CheckResult {
    CheckResult {
        verdict_streams: Default::default(),
        fault: None,
        classification: None,
        acceptance_id: "AC-X".into(),
        exit_code: exit,
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
        operational_error: None,
        environment_note: None,
    }
}

/// Run `command` exactly as a check site does: on the stdin of the
/// executing shell, with no environment.
#[cfg(unix)]
fn run(command: &str) -> CheckResult {
    let mut shell = archon_shell::spawn::command(archon_shell::resolve_posix_shell());
    let mut child =
        archon_shell::spawn::replace_environment(&mut shell, std::iter::empty::<(&str, &str)>())
            .arg("-s")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the executing shell starts");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(command.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let mut ran = result(
        output.status.code(),
        &String::from_utf8_lossy(&output.stderr),
    );
    ran.stdout = output.stdout;
    ran
}

#[cfg(unix)]
#[test]
fn the_executing_shell_rejects_a_trap_then_parse_error_text_with_its_line_554() {
    let error = shell_parse_error(TRAP_THEN_PARSE_ERROR).expect("the shell rejects it");
    assert!(
        error.contains("yntax error") && error.contains('7'),
        "{error}"
    );
    assert_eq!(
        shell_parse_error("set -eu\nif true; then echo ok; fi\n"),
        None
    );
    let finding = parse_finding("AC-X", &error);
    assert!(
        finding.contains("check 'AC-X'") && finding.contains(&error),
        "{finding}"
    );
}

/// A negative test of malformed input: a child shell rejects it on stdin,
/// and the check asserts exactly that. Its stderr carries the child's
/// syntax error, which no rule may read as the check's own.
const CHILD_REJECTS_INPUT: &str = "printf 'if then\\n' | sh -s; test $? -eq 2\n";

/// Parse-only mode never runs an alias definition, so it rejects a text
/// that runs fine.
const ALIAS_BLOCK: &str = "alias begin='if true; then'\nbegin\ntest 1 -eq 1\nfi\n";

/// Codex review cases: a valid check whose stderr holds a child shell's
/// syntax error is a pass, and a text that defines aliases is never
/// rejected by parse-only mode.
#[cfg(unix)]
#[test]
fn a_child_shells_syntax_error_and_an_alias_block_are_still_passes_554() {
    for command in [CHILD_REJECTS_INPUT, ALIAS_BLOCK] {
        let mut ran = run(command);
        assert_eq!(ran.exit_code, Some(0), "{command}: {ran:?}");
        assert_eq!(
            classify_check_run(command, &ran),
            CheckRunClass::Passed,
            "{command}"
        );
        ran.classify_raw(command);
        assert!(ran.classification.as_ref().unwrap().passed, "{command}");
    }
    assert_eq!(shell_parse_error(ALIAS_BLOCK), None);
    assert_eq!(
        shell_parse_error("shopt -s extglob\ncase x in @(x|y)) true;; esac\n"),
        None
    );
    assert_eq!(shell_parse_error(CHILD_REJECTS_INPUT), None);
    assert!(changes_parsing("set -o posix\n") && !changes_parsing("set -o pipefail\nalias_x=1\n"));
    assert!(shell_parse_error(&format!("set -o pipefail\n{TRAP_THEN_PARSE_ERROR}")).is_some());
    // The R11 text still fails the preflight: that is its one gate.
    assert!(shell_parse_error(TRAP_THEN_PARSE_ERROR).is_some());
}

/// Issue 554, stage `verify_command` (`sh -c`): a text its shell rejects is
/// never run and never accepted, whatever exit status a run would give.
#[cfg(unix)]
#[test]
fn a_verify_command_its_shell_cannot_parse_is_rejected_with_the_shell_error_554() {
    let root = tempfile::tempdir().unwrap();
    let error = crate::acceptance::run_verify_command(root.path(), Some(TRAP_THEN_PARSE_ERROR))
        .expect_err("a text the shell rejects is no pass");
    assert!(
        error.contains("does not parse") && error.contains("yntax error"),
        "{error}"
    );
}

/// Codex review cases for `verify_command`: a negative test of a nested
/// shell and an alias block are accepted.
#[cfg(unix)]
#[test]
fn a_verify_command_with_a_nested_shell_error_or_an_alias_block_is_accepted_554() {
    let root = tempfile::tempdir().unwrap();
    for command in ["! sh -c 'if then'", ALIAS_BLOCK] {
        assert_eq!(
            crate::acceptance::run_verify_command(root.path(), Some(command)),
            Ok(()),
            "{command}"
        );
    }
}
