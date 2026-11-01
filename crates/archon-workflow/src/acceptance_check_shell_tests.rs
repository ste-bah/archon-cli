//! Issue 554: a check text the executing shell rejects, and a run whose own
//! shell reported a syntax error, are never a pass.

use super::*;
use crate::acceptance_check_crash::{CheckRunClass, classify_check_run};

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

/// End to end: the R11 text run as a site runs it is never a pass, whatever
/// exit status this platform's shell gives it.
#[cfg(unix)]
#[test]
fn a_trap_then_parse_error_check_run_is_never_a_pass_554() {
    let mut ran = run(TRAP_THEN_PARSE_ERROR);
    let class = classify_check_run(TRAP_THEN_PARSE_ERROR, &ran);
    assert!(
        matches!(class, CheckRunClass::ScriptDefect(_)),
        "exit {:?}: {class:?}",
        ran.exit_code
    );
    ran.classify_raw(TRAP_THEN_PARSE_ERROR);
    let classification = ran.classification.as_ref().unwrap();
    assert!(!classification.passed, "{classification:?}");
}

#[test]
fn exit_zero_with_the_checks_own_shell_syntax_error_is_a_script_defect_554() {
    let cases: &[(&str, &str)] = &[
        // The recorded R11 baseline: exit 0 from the trap.
        (
            TRAP_THEN_PARSE_ERROR,
            "/bin/sh: line 7: syntax error near unexpected token `fi'\n",
        ),
        // Text the check evaluated itself (bash, dash).
        (
            "eval \"$plan\"\necho done\n",
            "/bin/sh: eval: line 2: syntax error near unexpected token `then'\n/bin/sh: eval: line 2: `if then'\n",
        ),
        (
            "eval \"$plan\"\n",
            "sh: 1: eval: Syntax error: \"then\" unexpected\n",
        ),
        // A file the check sources by that path.
        (
            ". ./lib/helpers.sh\nhelper_ok\n",
            "./lib/helpers.sh: line 3: syntax error near unexpected token `fi'\n",
        ),
    ];
    for (command, stderr) in cases {
        let ran = result(Some(0), stderr);
        assert!(
            matches!(
                classify_check_run(command, &ran),
                CheckRunClass::ScriptDefect(_)
            ),
            "passed: {stderr}"
        );
        let mut classified = ran.clone();
        classified.classify_raw(command);
        assert!(
            !classified.classification.as_ref().unwrap().passed,
            "{stderr}"
        );
        assert!(classified.reports_shell_syntax_error(), "{stderr}");
    }
}

#[test]
fn exit_zero_with_a_syntax_error_the_check_did_not_cause_stays_a_pass_554() {
    let cases: &[(&str, &str)] = &[
        // A product script run by path, not sourced.
        (
            "sh ./scripts/verify.sh || true\n",
            "./scripts/verify.sh: line 3: syntax error near unexpected token `then'\n",
        ),
        // A nested `sh -c` the check expects to fail.
        (
            "! sh -c 'if then'\n",
            "sh: -c: line 0: syntax error near unexpected token `then'\n",
        ),
        // Not a shell's line at all.
        (
            "./tool lint bad.yml\n",
            "bad.yml:3: syntax error in mapping\n",
        ),
    ];
    for (command, stderr) in cases {
        let ran = result(Some(0), stderr);
        assert_eq!(
            classify_check_run(command, &ran),
            CheckRunClass::Passed,
            "{stderr}"
        );
        assert!(!ran.reports_shell_syntax_error(), "{stderr}");
    }
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

#[test]
fn a_verify_command_that_exits_0_after_its_own_shell_syntax_error_is_no_pass_554() {
    let report = |stderr: &str| crate::acceptance::VerifyCommandReport {
        command: "./verify".into(),
        exit_code: Some(0),
        stdout: String::new(),
        stderr: stderr.into(),
        environment_note: None,
        parse_error: None,
    };
    for stderr in [
        "sh: -c: line 6: syntax error near unexpected token `fi'\n",
        "/usr/bin/sh: eval: line 0: syntax error near unexpected token `then'\n",
        "sh: 1: Syntax error: \"fi\" unexpected\n",
    ] {
        let found = report(stderr);
        assert!(!found.success(), "{stderr}");
        assert!(found.failure_reason().contains("syntax error"), "{stderr}");
    }
    assert!(report("./tool: line 3: syntax error in its own input\n").success());
    assert!(report("").success());
}
