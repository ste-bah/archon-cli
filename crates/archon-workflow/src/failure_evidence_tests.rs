use super::*;

const BUDGET: usize = 4000;

/// `cargo run` noise: warning blocks with source gutters and notes.
fn warning_block(n: usize) -> String {
    format!(
        "warning: unused variable: `error_{n}`\n  --> src/lib_{n}.rs:{n}:9\n   |\n{n:>3} |     let error_{n} = Err(Error::Unknown);\n   |         ^^^^^^^ help: prefix it with an underscore\n   = note: `#[warn(unused_variables)]` on by default\n\n"
    )
}

/// The tail the host used to keep, then the JS head-slice of it.
fn old_evidence(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut start = text.len().saturating_sub(4000);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let tail = format!("[truncated]\n{}", &text[start..]);
    tail.chars().take(1200).collect()
}

#[test]
fn three_hundred_warning_lines_then_the_error_keeps_the_error() {
    let mut out = String::new();
    for n in 0..300 {
        out.push_str(&format!("warning: unused import `x{n}` in module m{n}\n"));
    }
    out.push_str("Error: X\n");
    let evidence = failure_evidence(out.as_bytes(), BUDGET);
    assert!(evidence.contains("Error: X"), "{evidence}");
    assert!(evidence.len() <= BUDGET);
    // The old tail-then-head-slice path lost it.
    assert!(!old_evidence(out.as_bytes()).contains("Error: X"));
}

#[test]
fn live_shape_cargo_warnings_then_the_program_error() {
    let mut out = String::new();
    for n in 0..60 {
        out.push_str(&warning_block(n));
    }
    out.push_str(
        "Error: unknown asset_class `unknown`: expected one of [\"future\", \"equity\"]\n",
    );
    let evidence = failure_evidence(out.as_bytes(), BUDGET);
    assert!(
        evidence.ends_with("expected one of [\"future\", \"equity\"]"),
        "{evidence}"
    );
    assert!(!old_evidence(out.as_bytes()).contains("unknown asset_class"));
    // No warning scaffolding is promoted to a failure line.
    assert!(!evidence.contains("earlier error lines"), "{evidence}");
}

#[test]
fn an_error_in_the_middle_followed_by_many_lines_is_kept_with_the_end() {
    let mut out = String::new();
    for n in 0..200 {
        out.push_str(&format!("progress step {n} of the ingest\n"));
    }
    out.push_str("thread 'main' panicked at src/ingest.rs:42:7: Error: X broke\n");
    for n in 0..400 {
        out.push_str(&format!("cleanup line {n} after the failure\n"));
    }
    out.push_str("exit sequence complete\n");
    let evidence = failure_evidence(out.as_bytes(), BUDGET);
    assert!(
        evidence
            .contains("\nthread 'main' panicked at src/ingest.rs:42:7: Error: X broke  [L201]\n"),
        "{evidence}"
    );
    assert!(evidence.ends_with("exit sequence complete"), "{evidence}");
    assert!(evidence.len() <= BUDGET, "{}", evidence.len());
    assert!(!old_evidence(out.as_bytes()).contains("Error: X"));
}

#[test]
fn many_failure_lines_stay_bounded_keep_the_first_and_the_last() {
    let mut out = String::new();
    for n in 0..2000 {
        out.push_str(&format!(
            "test case_{n} ... FAILED with a long explanation {}\n",
            "x".repeat(40)
        ));
        out.push_str(&format!("   detail {n}\n"));
    }
    out.push_str("test result: FAILED. 0 passed; 2000 failed\n");
    let evidence = failure_evidence(out.as_bytes(), BUDGET);
    assert!(evidence.len() <= BUDGET, "{}", evidence.len());
    assert!(evidence.contains("case_0 ... FAILED"), "{evidence}");
    assert!(evidence.contains("more error lines omitted"), "{evidence}");
    assert!(evidence.ends_with("2000 failed"), "{evidence}");
}

#[test]
fn a_short_output_is_passed_whole() {
    let out = "warning: a\nError: short\n";
    assert_eq!(
        failure_evidence(out.as_bytes(), BUDGET),
        "warning: a\nError: short"
    );
}

#[test]
fn one_huge_line_keeps_its_end_and_the_budget() {
    let out = format!("{}Error: at the very end\n", "y".repeat(20_000));
    let evidence = failure_evidence(out.as_bytes(), BUDGET);
    assert!(evidence.ends_with("Error: at the very end"), "{evidence}");
    assert!(evidence.len() <= BUDGET);
}

#[test]
fn a_small_budget_is_never_exceeded_and_multibyte_text_is_safe() {
    let out = format!(
        "{}Error: é boom\n{}",
        "é noise\n".repeat(500),
        "ü tail\n".repeat(300)
    );
    for budget in [50, 400, 1200] {
        let evidence = failure_evidence(out.as_bytes(), budget);
        assert!(evidence.len() <= budget, "{budget}: {}", evidence.len());
    }
    for budget in [400, 420, 1200] {
        let evidence = failure_evidence(out.as_bytes(), budget);
        assert!(evidence.contains("Error: é boom"), "{budget}: {evidence}");
        assert!(evidence.ends_with("ü tail"), "{budget}: {evidence}");
    }
}

#[test]
fn failure_lines_are_detected_across_languages_and_warnings_are_not() {
    for line in [
        "error[E0425]: cannot find value `x` in this scope",
        "Error: unknown asset_class",
        "ERROR root: connection refused",
        "Traceback (most recent call last):",
        "ValueError: invalid literal for int()",
        "Exception in thread \"main\" java.lang.NullPointerException",
        "thread 'main' panicked at src/main.rs:3:5:",
        "panic: runtime error: index out of range",
        "AssertionError: expected 3 got 4",
        "assertion `left == right` failed",
        "--- FAIL: TestIngest (0.01s)",
        "FAILED tests/test_x.py::test_y - assert 1 == 2",
        "not ok 3 - parses dates",
        "fatal: not a git repository",
        "Segmentation fault (core dumped)",
        "Uncaught TypeError: x is undefined",
        "bash: line 1: jq: command not found",
        "sh: 1: foo: not found",
        "cat: data.csv: No such file or directory",
        "bash: ./run.sh: Permission denied",
        "make: *** [Makefile:3: test] Error 2",
        "src/a.ts(3,5): error TS2322: Type 'string' is not assignable",
        "Exception: boom",
        "./main.go:12:3: undefined: foo",
        "x_test.go:12: expected 3, got 4",
        "    x_test.go:12: expected 3, got 4",
        "npm ERR! code ELIFECYCLE",
        "Undefined symbols for architecture arm64:",
        "  left: 3",
        "Killed",
    ] {
        assert!(is_failure_line(line), "{line}");
    }
    for line in [
        "warning: unused variable: `error`",
        "   |     let e = Err(Error::Unknown);",
        "12 |     assert_eq!(a, b);",
        "  --> src/error.rs:3:1",
        "   = note: `#[warn(dead_code)]` on by default",
        "Compiling ingest v0.1.0",
        "Finished `dev` profile in 3.2s",
        "test result: ok. 12 passed; 0 failed; 0 ignored",
        "test tests::asserts_order ... ok",
        "src/x.rs:3:5: warning: unused import",
        "[output: 9 lines, 900 bytes; earlier error lines, then its last 3 lines]",
        "[4 more error lines omitted]",
        "/usr/lib/x.py:42: DeprecationWarning: old api",
        "127.0.0.1:8080: connected",
    ] {
        assert!(!is_failure_line(line), "{line}");
    }
}

#[test]
fn a_failure_keeps_the_location_line_that_follows_it() {
    let mut out =
        String::from("error[E0425]: cannot find value `x` in this scope\n  --> src/lib.rs:3:5\n");
    for n in 0..400 {
        out.push_str(&format!("   Compiling crate_{n} v0.1.0\n"));
    }
    let evidence = failure_evidence(out.as_bytes(), BUDGET);
    assert!(
        evidence.contains(
            "error[E0425]: cannot find value `x` in this scope  [L1]\n  --> src/lib.rs:3:5\n"
        ),
        "{evidence}"
    );
}

/// A small budget keeps its structure: the earlier failure line, the
/// marker, and the end, with a long last line clipped rather than
/// crowding the rest out.
#[test]
fn a_small_budget_keeps_the_error_line_and_a_long_last_line() {
    let mut out = String::from("Error: missing field asset_class\n");
    for n in 0..50 {
        out.push_str(&format!("warning: unused variable `v{n}`\n"));
    }
    out.push_str(&format!("{}\n", "z".repeat(600)));
    for budget in [400, 420, 800] {
        let evidence = failure_evidence(out.as_bytes(), budget);
        assert!(evidence.len() <= budget, "{budget}: {}", evidence.len());
        assert!(
            evidence.starts_with("[output: 52 lines"),
            "{budget}: {evidence}"
        );
        assert!(
            evidence.contains("Error: missing field asset_class  [L1]"),
            "{budget}: {evidence}"
        );
        assert!(evidence.ends_with("zzz"), "{budget}: {evidence}");
    }
}

/// The excerpt of an excerpt keeps the failure line and does not promote
/// its own markers to failure lines.
#[test]
fn re_excerpting_an_excerpt_keeps_the_failure() {
    let mut out = String::from("thread 'main' panicked at src/a.rs:1:1: boom\n");
    for n in 0..500 {
        out.push_str(&format!("line {n}\n"));
    }
    let first = failure_evidence(out.as_bytes(), 4000);
    let brief = failure_evidence(first.as_bytes(), 400);
    assert!(brief.contains("panicked at src/a.rs:1:1: boom"), "{brief}");
    assert!(!brief.contains("lines]  [L1]"), "{brief}");
    assert!(brief.len() <= 400);
}
