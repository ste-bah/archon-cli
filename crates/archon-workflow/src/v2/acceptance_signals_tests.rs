//! Batch E2: only the failure's own locations are read from a check's output.

use super::*;

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "src/a.rs",
        "src/b.rs",
        "src/c.rs",
        "tests/t.py",
        "web/app.js",
    ] {
        let target = dir.path().join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, "//\n").unwrap();
    }
    dir
}

fn located(text: &str) -> Vec<String> {
    let dir = repo();
    failure_locations(text, dir.path(), 6)
}

#[test]
fn an_error_diagnostics_primary_span_is_read_and_its_notes_and_warnings_are_not() {
    let text = "warning: unused\n --> src/c.rs:1:1\n  |\n1 | x\n  |\n\n\
                error[E0425]: cannot find value `y`\n --> src/a.rs:3:5\n  |\n3 | y\n  | ^\n\
                note: defined here\n --> src/b.rs:9:1\n\n\
                error: could not compile `a` due to 1 previous error\n";
    assert_eq!(located(text), ["src/a.rs"]);
}

#[test]
fn location_led_lines_count_only_at_error_level() {
    let text = "src/a.rs:1:1: warning: unused\nsrc/b.rs:2:1: note: here\n\
                src/c.rs:3: needle\nsrc/a.rs:4: lint W0612 unused\n";
    assert!(located(text).is_empty());
    assert_eq!(located("src/b.rs:3:5: error: nope\n"), ["src/b.rs"]);
    assert_eq!(located("tests/t.py:12: AssertionError\n"), ["tests/t.py"]);
    assert_eq!(located("tests/t.py:8: in test_x\n"), ["tests/t.py"]);
}

#[test]
fn panics_and_stack_frames_are_read() {
    let text = "thread 'main' panicked at src/a.rs:41:9:\nboom\nstack backtrace:\n   \
                3: crate::f\n             at ./src/b.rs:10:5\n";
    assert_eq!(located(text), ["src/b.rs", "src/a.rs"]);
    let old = "thread 'main' panicked at 'boom', src/c.rs:2:3\n";
    assert_eq!(located(old), ["src/c.rs"]);
    let python = "Traceback (most recent call last):\n  File \"tests/t.py\", line 14, in <module>\n\
                  \x20   assert x\nAssertionError\n  File \"<string>\", line 3\n";
    assert_eq!(located(python), ["tests/t.py"]);
    let js = "TypeError: x is undefined\n    at run (web/app.js:10:5)\n";
    assert_eq!(located(js), ["web/app.js"]);
}

#[test]
fn a_span_whose_header_was_truncated_away_is_not_read() {
    let text = "[truncated]\nementation\n90 |     pub fn f() {}\n   |    ^\n\
                \x20 --> src/a.rs:90:19\n\nError: unknown asset_class `unknown`\n";
    assert!(located(text).is_empty());
}
