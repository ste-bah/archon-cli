use super::{cargo_package, failing_tests, is_cargo_test_command, tail};

const LIBTEST_OUTPUT: &str = "\
running 4 tests
test alpha::beta::passes ... ok
test alpha::beta::breaks_one ... FAILED
test alpha::gamma::breaks_two ... FAILED
test alpha::delta::slow ... ok

failures:

---- alpha::beta::breaks_one stdout ----
thread 'alpha::beta::breaks_one' panicked at src/alpha/beta.rs:10:5:
assertion failed: false

---- alpha::gamma::breaks_two stdout ----
thread 'alpha::gamma::breaks_two' panicked at src/alpha/gamma.rs:4:5:
boom


failures:
    alpha::beta::breaks_one
    alpha::gamma::breaks_two

test result: FAILED. 2 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
";

#[test]
fn result_lines_and_the_failures_block_name_the_same_tests_once() {
    assert_eq!(
        failing_tests(LIBTEST_OUTPUT),
        vec![
            "alpha::beta::breaks_one".to_string(),
            "alpha::gamma::breaks_two".to_string()
        ]
    );
}

#[test]
fn a_harness_killed_before_its_block_is_read_from_the_result_lines_alone() {
    let cut = "test a::b ... ok\ntest a::c ... FAILED\ntest a::d ... FAILED\n";
    assert_eq!(
        failing_tests(cut),
        vec!["a::c".to_string(), "a::d".to_string()]
    );
}

#[test]
fn prose_naming_a_test_and_detail_headers_are_not_failures() {
    let prose = "\
the test a::b ... FAILED yesterday, said someone
---- a::b stdout ----
failures:
not indented so not a name
test ok_one ... ok
test result: ok. 1 passed; 0 failed
";
    assert!(
        failing_tests(prose).is_empty(),
        "{:?}",
        failing_tests(prose)
    );
}

#[test]
fn a_timing_suffix_on_the_verdict_still_reads_as_failed() {
    assert_eq!(
        failing_tests("test x::y ... FAILED (1.2s)\n"),
        vec!["x::y".to_string()]
    );
}

#[test]
fn nextest_verdict_lines_are_read_too() {
    let out = "        FAIL [   0.021s] archon-workflow v2::write::t::one\n        PASS [   0.001s] archon-workflow v2::write::t::two\n";
    assert_eq!(failing_tests(out), vec!["v2::write::t::one".to_string()]);
}

#[test]
fn cargo_commands_are_recognised_and_their_package_read() {
    assert!(is_cargo_test_command("cargo test -p archon-workflow write"));
    assert!(is_cargo_test_command("CARGO_X=1 cargo nextest run -p foo"));
    assert!(!is_cargo_test_command("pytest tests/"));
    assert_eq!(
        cargo_package("cargo test -p archon-workflow write"),
        Some("archon-workflow".into())
    );
    assert_eq!(
        cargo_package("cargo test --package foo"),
        Some("foo".into())
    );
    assert_eq!(
        cargo_package("cargo test --package=bar x"),
        Some("bar".into())
    );
    assert_eq!(cargo_package("cargo test --bin archon x"), None);
}

#[test]
fn the_tail_keeps_the_last_forty_lines() {
    let many: String = (0..100).map(|n| format!("line {n}\n")).collect();
    let kept = tail(&many);
    assert_eq!(kept.len(), 40);
    assert_eq!(kept.first().map(String::as_str), Some("line 60"));
    assert_eq!(kept.last().map(String::as_str), Some("line 99"));
}

/// Issue-64: error locations are read by file, relative to the worktree;
/// warnings, notes and foreign absolute paths are not failures of the tree.
#[test]
fn diagnostic_files_reads_error_locations_and_fmt_diffs_relative_to_the_worktree() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("ws");
    std::fs::create_dir_all(&worktree).unwrap();
    let output = format!(
        "warning: unused variable\n --> src/only_warned.rs:1:1\n  |\n\
         error[E0433]: failed to resolve\n  --> crates/a/src/lib.rs:12:5\n   |\n\
         12 |     use x::y;\nnote: see also\n    --> crates/a/src/note.rs:1:1\n\
         error: redundant closure\n --> {}/crates/b/src/gate.rs:4:9\n\
         error: unused import\n --> /home/other/.cargo/registry/src/dep/lib.rs:1:1\n\
         Diff in {}/crates/c/src/fmt.rs:30:\n\
         Diff in crates/c/src/rel.rs:2:\n\
         error: could not compile `a` due to 3 previous errors\n\
         error: aborting\n --> crates\\a\\src\\win.rs:1:1\n",
        worktree.display(),
        std::fs::canonicalize(&worktree).unwrap().display()
    );
    assert_eq!(
        super::diagnostic_files(&output, &worktree),
        vec![
            "crates/a/src/lib.rs".to_string(),
            "crates/a/src/win.rs".to_string(),
            "crates/b/src/gate.rs".to_string(),
            "crates/c/src/fmt.rs".to_string(),
            "crates/c/src/rel.rs".to_string(),
        ]
    );
    assert!(super::diagnostic_files("test a ... FAILED\n", &worktree).is_empty());
}
