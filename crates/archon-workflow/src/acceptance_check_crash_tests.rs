//! The script-defect rule over interpreter output recorded from real runs.

use super::*;

fn result(exit: Option<i32>, stderr: &str) -> CheckResult {
    CheckResult {
        classification: None,
        acceptance_id: "AC-X".into(),
        exit_code: exit,
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
        operational_error: None,
    }
}

fn defect(command: &str, exit: i32, stderr: &str) -> bool {
    matches!(
        classify_check_run(command, &result(Some(exit), stderr)),
        CheckRunClass::ScriptDefect(_)
    )
}

const HEREDOC: &str = "python3 - <<'PY'\nimport json, sys\ndef run(env_extra, args, d):\n    return {}\nrep = run({}, [\"x\"])\nassert rep['ok'], 'not ok'\nPY\n";

/// The shape recorded for a live re-authored check: its own helper called
/// with one argument too few, innermost frame the heredoc's call line.
const LIVE_CRASH: &str = "Traceback (most recent call last):\n  File \"<stdin>\", line 4, in <module>\nTypeError: run() missing 1 required positional argument: 'd'\n";

#[test]
fn acceptance_crash_in_a_helper_the_check_defines_is_a_script_defect() {
    let class = classify_check_run(HEREDOC, &result(Some(1), LIVE_CRASH));
    let CheckRunClass::ScriptDefect(found) = class else {
        panic!("{class:?}")
    };
    assert_eq!(found.interpreter, "python");
    assert!(
        found.signal.contains("run() missing 1 required"),
        "{found:?}"
    );
    assert!(
        found
            .finding("AC-X", LIVE_CRASH)
            .contains("keep every assertion")
    );
}

#[test]
fn acceptance_product_failures_are_never_script_defects() {
    let cases: &[(&str, i32, &str)] = &[
        // The check's own assertion said the criterion is false.
        (
            HEREDOC,
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 6, in <module>\nAssertionError: not ok\n",
        ),
        // sys.exit(message): an expected-failure signal, no traceback.
        (
            HEREDOC,
            1,
            "fetch-native exited 2 for ['x']: error: no such subcommand\n",
        ),
        // The product returned something the check could not use: data-dependent.
        (
            HEREDOC,
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 6, in <module>\nKeyError: 'ok'\n",
        ),
        (
            HEREDOC,
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 6, in <module>\nTypeError: unsupported operand type(s) for -: 'NoneType' and 'float'\n",
        ),
        (
            HEREDOC,
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 6, in <module>\nAttributeError: 'NoneType' object has no attribute 'get'\n",
        ),
        // Innermost frame is a library the product output broke.
        (
            HEREDOC,
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 2, in <module>\n  File \"/usr/lib/python3.9/json/decoder.py\", line 353, in raw_decode\n    obj, end = self.scan_once(s, idx)\njson.decoder.JSONDecodeError: Expecting value: line 1 column 1 (char 0)\n",
        ),
        // Innermost frame is a product file the check imported.
        (
            HEREDOC,
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 5, in <module>\n  File \"/repo/src/tool.py\", line 9, in run\n    helper()\nNameError: name 'helper' is not defined\n",
        ),
        // A product API with a changed signature: not a function the check defines.
        (
            HEREDOC,
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 5, in <module>\nTypeError: fetch() missing 1 required positional argument: 'symbol'\n",
        ),
        // A name the check never wrote (e.g. exec'd product text).
        (
            HEREDOC,
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 5, in <module>\nNameError: name 'product_symbol' is not defined\n",
        ),
        // Anything printed after the exception line: not a bare crash.
        (
            HEREDOC,
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 4, in <module>\nTypeError: run() missing 1 required positional argument: 'd'\ncleanup: removed temp dir\n",
        ),
        // The crash signature with an exit code Python never uses for it.
        (HEREDOC, 2, LIVE_CRASH),
        // No python in the check: the traceback is some child's.
        ("./bin/tool --check", 1, LIVE_CRASH),
        // A compile() of product text in <string>, text not the check's.
        (
            HEREDOC,
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 5, in <module>\n  File \"<string>\", line 1\n    generated = (\n    ^\nSyntaxError: unexpected EOF while parsing\n",
        ),
        // A missing product binary or tool is not the check's own helper.
        (
            "set -e\nmytool --version\n",
            127,
            "sh: line 2: mytool: command not found\n",
        ),
        // A product shell script's syntax error, run by path or through eval.
        (
            "sh ./scripts/verify.sh\n",
            2,
            "./scripts/verify.sh: line 3: syntax error near unexpected token `then'\n./scripts/verify.sh: line 3: `if then fi'\n",
        ),
        (
            "eval \"$(./bin/tool env)\"\n",
            2,
            "sh: eval: line 1: syntax error near unexpected token `)'\nsh: eval: line 1: `x=)'\n",
        ),
        // A shell syntax error whose echoed text is not the check's.
        (
            "bash -s < generated.sh\n",
            2,
            "bash: line 4: syntax error near unexpected token `fi'\nbash: line 4: `  fi fi'\n",
        ),
        // The crashing line is not a call of the check's helper at all.
        (
            HEREDOC,
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 5, in <module>\nTypeError: run() missing 1 required positional argument: 'd'\n",
        ),
        // A name the product was meant to export through a star import.
        (
            "python3 - <<'PY'\nfrom tool import *\nassert parse_config(\"a=1\") == {\"a\": \"1\"}\nPY\n",
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 2, in <module>\nNameError: name 'parse_config' is not defined\n",
        ),
        // A product method whose name collides with a helper the check defines.
        (
            "python3 - <<'PY'\nimport tool\ndef fetch(url):\n    return url\nc = tool.Client()\nassert c.fetch(timeout=5) == 1\nPY\n",
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 5, in <module>\nTypeError: fetch() got an unexpected keyword argument 'timeout'\n",
        ),
        (
            "python3 - <<'PY'\nimport tool\ndef fetch(url):\n    return url\nc = tool.Client()\nassert c.fetch(timeout=5) == 1\nPY\n",
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 5, in <module>\nTypeError: Client.fetch() got an unexpected keyword argument 'timeout'\n",
        ),
        // The product exec'd text: a product frame sits between the check's
        // frame and the innermost <string> frame.
        (
            "python3 - <<'PY'\nimport tool\ntool.apply_rules(\"x = threshold + 1\")\nprint(\"threshold applied\")\nPY\n",
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 2, in <module>\n  File \"/tmp/fp/tool/__init__.py\", line 10, in apply_rules\n    exec(compile(rule, \"<string>\", \"exec\"), env)\n  File \"<string>\", line 1, in <module>\nNameError: name 'threshold' is not defined\n",
        ),
        // The product's output unpacked into the check's own helper.
        (
            "python3 - <<'PY'\nimport subprocess\ndef check(name, count):\n    assert int(count) > 0\nrow = subprocess.run(['./tool'], capture_output=True, text=True).stdout\ncheck(*row.split())\nPY\n",
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 5, in <module>\nTypeError: check() takes 2 positional arguments but 3 were given\n",
        ),
        (
            "python3 - <<'PY'\nimport json, subprocess\ndef check(name, count):\n    assert count > 0\nout = subprocess.run(['./tool'], capture_output=True, text=True).stdout\ncheck(**json.loads(out))\nPY\n",
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 5, in <module>\nTypeError: check() got an unexpected keyword argument 'extra'\n",
        ),
        // A name bound only on a branch the product's output skipped.
        (
            "python3 - <<'PY'\nimport subprocess\nout = subprocess.run(['./tool'], capture_output=True, text=True).stdout.strip()\nif out == 'ok':\n    status = 'ok'\nassert status == 'ok'\nPY\n",
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 5, in <module>\nNameError: name 'status' is not defined\n",
        ),
        (
            "python3 - <<'PY'\ndef f():\n    print(total)\n    total = 1\nf()\nPY\n",
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 4, in <module>\n  File \"<stdin>\", line 2, in f\nUnboundLocalError: local variable 'total' referenced before assignment\n",
        ),
        // A product-generated script run through a nested `bash -c`.
        (
            "bash -c \"$(printf 'if true; then\\n echo hi\\n')\"",
            2,
            "bash: -c: line 2: syntax error: unexpected end of file\n",
        ),
        // A stdin-fed nested shell whose script is not the check's text.
        (
            "./bin/tool emit | sh -s\n",
            2,
            "sh: line 2: syntax error: unexpected end of file\n",
        ),
    ];
    for (command, exit, stderr) in cases {
        assert!(
            !defect(command, *exit, stderr),
            "misread as a script defect: {stderr}"
        );
    }
}

#[test]
fn acceptance_script_defects_are_recognized_across_interpreters() {
    let cases: &[(&str, i32, &str)] = &[
        // python -c, undefined name the check wrote.
        (
            "python3 -c 'print(undefined_x)'",
            1,
            "Traceback (most recent call last):\n  File \"<string>\", line 1, in <module>\nNameError: name 'undefined_x' is not defined\n",
        ),
        // Python 3.11+ echoes -c source with carets.
        (
            "python3 -c 'print(undefined_x)'",
            1,
            "Traceback (most recent call last):\n  File \"<string>\", line 1, in <module>\n    print(undefined_x)\n          ^^^^^^^^^^^\nNameError: name 'undefined_x' is not defined. Did you mean: 'undefined'?\n",
        ),
        // A heredoc that never closes its bracket.
        (
            "python3 - <<'PY'\nx = (\nPY\n",
            1,
            "  File \"<stdin>\", line 2\n    \n    ^\nSyntaxError: unexpected EOF while parsing\n",
        ),
        (
            "python3 - <<'PY'\nif True:\nprint(1)\nPY\n",
            1,
            "  File \"<stdin>\", line 2\n    print(1)\n    ^\nIndentationError: expected an indented block\n",
        ),
        // Too many arguments to a method of a class the check defines.
        (
            "python3 - <<'PY'\nclass Lane:\n    def __init__(self, a):\n        pass\nLane(1, 2)\nPY\n",
            1,
            "Traceback (most recent call last):\n  File \"<stdin>\", line 4, in <module>\nTypeError: Lane.__init__() takes 2 positional arguments but 3 were given\n",
        ),
        // sh (bash in POSIX mode), bash, and dash syntax errors.
        (
            "echo a\nif then fi\n",
            2,
            "sh: line 2: syntax error near unexpected token `then'\nsh: line 2: `if then fi'\n",
        ),
        (
            "echo a\nif then fi\n",
            2,
            "bash: line 2: syntax error near unexpected token `then'\nbash: line 2: `if then fi'\n",
        ),
        (
            "echo a\nif then fi\n",
            2,
            "sh: 2: Syntax error: \"then\" unexpected\n",
        ),
        // A helper the check defines but calls before defining it.
        (
            "check_rows data.jsonl\ncheck_rows() { test -s \"$1\"; }\n",
            127,
            "sh: line 1: check_rows: command not found\n",
        ),
        (
            "function lane_ok { true; }\nlane_okk\nlane_ok\nlane_okk\n",
            127,
            "sh: 4: lane_okk: not found\n",
        ),
    ];
    for (command, exit, stderr) in &cases[..cases.len() - 1] {
        assert!(defect(command, *exit, stderr), "missed: {stderr}");
    }
    // A misspelled helper is NOT one the check defines: conservative.
    let (command, exit, stderr) = cases[cases.len() - 1];
    assert!(!defect(command, exit, stderr));
}

#[test]
fn acceptance_passes_timeouts_and_operational_errors_are_not_script_defects() {
    assert_eq!(
        classify_check_run(HEREDOC, &result(Some(0), LIVE_CRASH)),
        CheckRunClass::Passed
    );
    assert_eq!(
        classify_check_run(HEREDOC, &result(None, LIVE_CRASH)),
        CheckRunClass::Failed
    );
    let mut errored = result(Some(1), LIVE_CRASH);
    errored.operational_error = Some("timed out".into());
    assert_eq!(classify_check_run(HEREDOC, &errored), CheckRunClass::Failed);
}
