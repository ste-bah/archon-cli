use super::*;

fn cases() -> Vec<(&'static str, &'static str, bool)> {
    vec![
        ("set -u; trap 'exit 0' EXIT; test -f \"$X\"", "X", true),
        ("set -u\ntrap 'exit 0' EXIT\ntest -f \\\n\"$X\"", "X", true),
        (
            "set -u\ntrap 'exit 0' EXIT\ncat <<EOF\n$X\nEOF\n",
            "X",
            true,
        ),
        (
            "set -u\ntrap 'exit 0' EXIT\nvalue=$(\n test -f \"$X\"\n)\n",
            "X",
            false,
        ),
        (
            "set -u; trap 'exit 0' EXIT; echo $(( COUNT + 0 ))",
            "COUNT",
            false,
        ),
        (
            "set -u; trap 'exit 0' EXIT; eval 'test -f $OUTX'",
            "OUTX",
            false,
        ),
        ("set -u; trap 'exit 0' EXIT; : \"${X:?msg}\"", "X", false),
        ("set -u; trap 'exit 0' EXIT; : \"${X?msg}\"", "X", false),
        (
            "set -u; X=value; sh -s <<'CHILD'\nset -u\necho $X\nCHILD\ntrue",
            "X",
            false,
        ),
    ]
}

#[test]
fn masked_aborts_never_pass_and_only_never_bound_reads_are_defects() {
    for (command, name, is_defect) in cases() {
        let message = if command.contains("${X") {
            "msg"
        } else {
            "unbound variable"
        };
        // The reported line deliberately differs from the read's physical line.
        let stderr = format!("/bin/bash: line 1: {name}: {message}\n");
        let mut ran = result(Some(0), &stderr);
        ran.classify_raw(command);
        assert!(!ran.passed(), "{command}");
        assert_eq!(
            matches!(
                classify_check_run(command, &ran),
                CheckRunClass::ScriptDefect(_)
            ),
            is_defect,
            "{command}"
        );
        ran.stderr.clear();
        assert!(!ran.passed(), "raw classification must survive redaction");
    }
}

#[test]
fn fatal_shapes_and_required_values_have_separate_attribution_rules() {
    for stderr in [
        "sh: line 12: X: unbound variable",
        "/bin/bash: line 1: X: parameter null or not set",
        "bash: line 1: X: parameter not set",
        "/bin/dash: 9: X: parameter not set",
    ] {
        assert_eq!(
            classify_execution("X=bound", Some(0), stderr, &[]),
            CheckRunClass::Failed
        );
        assert_eq!(
            classify_execution("set -u; echo $X", Some(0), stderr, &["X"]),
            CheckRunClass::Failed
        );
    }
    assert_eq!(
        classify_execution(
            "value=$(./tool); : \"${value:?empty}\"",
            Some(0),
            "sh: line 1: value: empty",
            &[]
        ),
        CheckRunClass::Failed
    );
    assert_eq!(
        classify_execution("true", Some(0), "tool.sh: line 1: X: unbound variable", &[]),
        CheckRunClass::Passed
    );
    assert_eq!(
        classify_execution("true", Some(0), "sh: line 1: X: ordinary message", &[]),
        CheckRunClass::Passed
    );
    assert_eq!(
        classify_execution("true", Some(1), "sh: line 1: X: unbound variable", &[]),
        CheckRunClass::Failed
    );
}

#[cfg(unix)]
#[test]
fn executing_shell_aborts_cannot_be_false_green() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    for (command, _, _) in cases() {
        let mut child = Command::new("/bin/bash")
            .arg("-s")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
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
        ran.classify_raw(command);
        assert!(!ran.passed(), "{command}: {output:?}");
    }
}
