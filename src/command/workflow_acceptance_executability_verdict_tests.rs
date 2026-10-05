//! Issue 328: which failed runs gave a verdict, on captured output shapes.

use archon_workflow::acceptance_scratch::CheckResult;

use super::{host_failure, no_verdict};

fn run(code: Option<i32>, stdout: &str, stderr: &str) -> CheckResult {
    CheckResult {
        acceptance_id: "AC-1".into(),
        exit_code: code,
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
        quota_walk_count: 0,
        operational_error: None,
        classification: None,
    }
}

const RUSTC: &str = "   Compiling lake v0.1.0 (/tmp/copy)\nerror[E0308]: mismatched types\n --> src/lib.rs:1:21\n  |\n1 | pub fn f() -> u32 { \"x\" }\n  |                     ^^^ expected `u32`, found `&str`\n\nerror: could not compile `lake` (lib) due to 1 previous error\n";

#[test]
fn a_tree_that_does_not_build_gives_no_verdict() {
    for (code, stderr) in [
        (101, RUSTC),
        (
            1,
            "src/main.c:3:5: error: use of undeclared identifier 'x'\n",
        ),
        (1, "lib/a.go:7:2: fatal error: missing.h: No such file\n"),
        (2, "src/a.ts(3,5): error TS2304: Cannot find name 'x'.\n"),
        (
            1,
            "Traceback (most recent call last):\n  File \"/tmp/copy/pkg/mod.py\", line 3\n    def (:\n        ^\nSyntaxError: invalid syntax\n",
        ),
    ] {
        let result = run(Some(code), "", stderr);
        let why = no_verdict("sh -c true", &result).unwrap_or_else(|| panic!("{stderr}"));
        assert!(why.contains("did not build"), "{why}");
        assert!(host_failure(&result).is_some(), "{stderr}");
    }
}

#[test]
fn a_program_that_could_not_start_or_a_run_without_status_gives_no_verdict() {
    for code in [126, 127] {
        let result = run(Some(code), "", "sh: 1: jq: not found\n");
        let why = no_verdict("jq . x.json", &result).expect("no verdict");
        assert!(why.contains(&format!("exit {code}")), "{why}");
    }
    for stderr in [
        "bash: line 1: python3.99: command not found\n",
        "/usr/bin/env: 'node': No such file or directory\n",
        "sh: 1: /opt/tool/bin/lint: not found\n",
        "bash: ./run.sh: /usr/bin/python9: bad interpreter: No such file or directory\n",
        "",
    ] {
        let result = run(Some(127), "", stderr);
        assert!(no_verdict("x", &result).is_some(), "{stderr}");
    }
    let killed = run(None, "", "");
    assert!(no_verdict("sleep 9", &killed).is_some());
}

#[test]
fn an_assertion_that_ran_is_a_verdict_even_beside_a_compiler_error() {
    for (code, stdout, stderr) in [
        (1, "", "expected ready, found pending\n"),
        (101, "test result: FAILED. 0 passed; 1 failed\n", RUSTC),
        (101, "", "thread 'main' panicked at src/lib.rs:4:5:\nboom\n"),
        (1, "", "AssertionError: 1 != 2\n"),
        (1, "FAILED tests/test_x.py::test_y\n", ""),
        // The deliverable is absent on this tree: a file of the tree that
        // is not there yet, or no source location.
        (127, "", "bash: scripts/new.sh: No such file or directory\n"),
        (127, "", "sh: 1: ./target/debug/tool: not found\n"),
        (126, "", "sh: 1: ./bin/tool: Permission denied\n"),
        (
            101,
            "",
            "error: no test target named `ingest` in `lake` package\n",
        ),
        (2, "", "error: unrecognized subcommand 'data'\n"),
        // A warning's span is not an error's.
        (
            1,
            "",
            "warning: unused variable\n --> src/lib.rs:2:9\n\nerror: missing deliverable\n",
        ),
    ] {
        let result = run(Some(code), stdout, stderr);
        assert_eq!(no_verdict("sh -c true", &result), None, "{stdout}{stderr}");
    }
}

#[test]
fn a_run_that_did_no_work_or_passed_is_no_host_failure() {
    let zero = run(Some(0), "running 0 tests\n", "");
    assert_eq!(no_verdict("cargo test x", &zero), None);
    let passed = run(Some(0), "ok\n", "");
    assert_eq!(no_verdict("true", &passed), None);
    let operational = CheckResult {
        operational_error: Some("timed out".into()),
        ..run(None, "", "")
    };
    assert_eq!(no_verdict("true", &operational), None, "reported as such");
}

#[test]
fn a_crash_in_the_checks_own_code_gives_no_verdict_but_is_no_host_failure() {
    let command = "python3 - <<'PY'\nundefined_helper()\nPY";
    let result = run(
        Some(1),
        "",
        "Traceback (most recent call last):\n  File \"<stdin>\", line 1, in <module>\nNameError: name 'undefined_helper' is not defined\n",
    );
    assert!(
        no_verdict(command, &result).is_some_and(|why| why.contains("crashed in its own python"))
    );
    assert_eq!(host_failure(&result), None, "a crash is the check's own");
}
