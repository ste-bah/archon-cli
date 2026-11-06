//! Real shell domain matrix and portable recordings of their diagnostics.
use crate::acceptance_check_crash::{CheckRunClass, classify_execution};

fn assert_class(command: &str, code: i32, stderr: &str, defect: bool, row: &str) {
    let class = classify_execution(command, Some(code), stderr, &[]);
    assert!(
        if defect {
            matches!(class, CheckRunClass::ScriptDefect(_))
        } else {
            matches!(class, CheckRunClass::Failed)
        },
        "{row}: code={code}, stderr={stderr:?}, class={class:?}, command={command:?}"
    );
}

#[test]
#[cfg(unix)]
fn real_shell_expansion_domain() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let cases = [
        ("same line", "echo \"$X\"", true),
        ("continuation", "echo \\\n\"$X\"", true),
        ("heredoc", "cat <<END\n$X\nEND\n", true),
        ("required message", ": \"${X:?msg}\"", false),
        ("required empty", ": \"${X:?}\"", false),
        ("eval", "eval 'test -f $X'", false),
        ("child", "sh -c 'set -u; echo \"$X\"'; :", false),
    ];
    for (shell, mode, multiline) in [
        ("bash", "-s", false),
        ("bash", "-c", false),
        ("bash", "-c", true),
        ("sh", "-s", false),
        ("sh", "-c", false),
        ("dash", "-s", false),
        ("dash", "-c", false),
    ] {
        // Only absence permits a skip; a present but broken binary fails the row.
        match Command::new(shell).args(["-c", ":"]).status() {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            result => assert!(result.expect("shell availability").success()),
        }
        for (case, body, defect) in cases {
            for trapped in [false, true] {
                let separator = if multiline || mode == "-s" {
                    "\n"
                } else {
                    "; "
                };
                let trap = if trapped {
                    format!("trap 'exit 0' EXIT{separator}")
                } else {
                    String::new()
                };
                let command = format!("set -u{separator}{trap}{body}");
                let mut process = Command::new(shell);
                process.arg(mode).env_clear().env("PATH", "/usr/bin:/bin");
                if mode == "-c" {
                    process.arg(&command);
                }
                let mut child = process
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .expect("run shell");
                if mode == "-s" {
                    child
                        .stdin
                        .take()
                        .unwrap()
                        .write_all(command.as_bytes())
                        .unwrap();
                } else {
                    drop(child.stdin.take());
                }
                let output = child.wait_with_output().unwrap();
                let stderr = String::from_utf8(output.stderr).unwrap();
                assert!(!stderr.is_empty(), "{shell} {mode} {case}");
                let row = format!("{shell} {mode} multiline={multiline} {case} trap={trapped}");
                assert_class(
                    &command,
                    output.status.code().expect("exit status"),
                    &stderr,
                    defect,
                    &row,
                );
            }
        }
    }
}

#[test]
fn recorded_real_shell_diagnostics() {
    // Recordings include macOS bash 3.2's one-line -c form (no line number),
    // bash stdin/multiline -c, and Debian/Ubuntu dash stdin/-c/eval.
    let ordinary = "set -u; echo \"$X\"";
    let required = "set -u; : \"${X:?msg}\"";
    let rows = [
        (ordinary, "/bin/sh: X: unbound variable", true),
        (required, "/bin/sh: X: msg", false),
        (
            "set -u; : \"${ID:?no id}\"",
            "/bin/dash: 1: ID: no id",
            false,
        ),
        (
            "set -u; : \"${ID:?}\"",
            "/bin/dash: 1: ID: parameter not set or null",
            false,
        ),
        (
            "set -u; eval 'test -f $OUTX'",
            "/bin/dash: 1: eval: OUTX: parameter not set",
            false,
        ),
        (ordinary, "bash: line 1: X: unbound variable", true),
        (ordinary, "bash: line 2: X: unbound variable", true),
        (ordinary, "sh: line 2: X: unbound variable", true),
        (ordinary, "/bin/dash: 1: X: parameter not set", true),
        (ordinary, "sh: 2: X: parameter not set", true),
        (required, "/bin/dash: 1: X: msg", false),
        ("set -u; : \"${X?msg}\"", "/bin/dash: 1: X: msg", false),
        (
            "set -u; : \"${X:?}\"",
            "/bin/dash: 1: X: parameter not set or null",
            false,
        ),
        (
            "set -u; : \"${X:?}\"",
            "/bin/sh: X: parameter null or not set",
            false,
        ),
        (
            "set -u; eval 'test -f $X'",
            "/bin/dash: 1: eval: X: parameter not set",
            false,
        ),
        (
            "sh -c 'set -u; echo \"$X\"'; :",
            "sh: 1: X: parameter not set",
            false,
        ),
    ];
    for (command, stderr, defect) in rows {
        for code in [0, 2] {
            assert_class(command, code, stderr, defect, "recorded");
        }
        // No executed text survives in some legacy/declarative captures.
        assert_class("", 0, stderr, false, "command unavailable");
    }
    assert!(matches!(
        classify_execution("echo ok", Some(0), "", &[]),
        CheckRunClass::Passed
    ));
    assert_class(
        ordinary,
        0,
        "/bin/sh: X: unbound variable",
        true,
        "never bound",
    );
    assert!(matches!(
        classify_execution(ordinary, Some(0), "/bin/sh: X: unbound variable", &["X"]),
        CheckRunClass::Failed
    ));
}
