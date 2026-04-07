//! Tests for TASK-PIPE-E05: Forbidden Pattern Scanner
//!
//! These tests verify:
//! - All forbidden patterns are detected (zero false negatives)
//! - Clean production code produces zero matches
//! - Test files are exempt from scanning
//! - Gate pass/fail logic based on match presence
//! - Multi-file scanning with correct aggregation
//! - Test file skipping in scan_files
//! - Correct 1-indexed line numbers
//! - Pattern names correctly identify triggered patterns

use archon_pipeline::coding::gates::{ForbiddenPatternScanner, Severity};

// ---------------------------------------------------------------------------
// 1. Forbidden pattern detection — zero false negatives
// ---------------------------------------------------------------------------

#[test]
fn detects_todo_comment() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "fn main() {\n    // TODO: fix this\n}\n";
    let matches = scanner.scan_content("src/lib.rs", content);
    assert!(
        matches.iter().any(|m| m.matched_text.contains("TODO")),
        "Expected TODO comment to be detected, got: {:?}",
        matches
    );
}

#[test]
fn detects_fixme_comment() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "fn main() {\n    // FIXME: broken\n}\n";
    let matches = scanner.scan_content("src/lib.rs", content);
    assert!(
        matches.iter().any(|m| m.matched_text.contains("FIXME")),
        "Expected FIXME comment to be detected, got: {:?}",
        matches
    );
}

#[test]
fn detects_hack_comment() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "fn main() {\n    // HACK: workaround\n}\n";
    let matches = scanner.scan_content("src/lib.rs", content);
    assert!(
        matches.iter().any(|m| m.matched_text.contains("HACK")),
        "Expected HACK comment to be detected, got: {:?}",
        matches
    );
}

#[test]
fn detects_xxx_comment() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "fn main() {\n    // XXX: dangerous\n}\n";
    let matches = scanner.scan_content("src/lib.rs", content);
    assert!(
        matches.iter().any(|m| m.matched_text.contains("XXX")),
        "Expected XXX comment to be detected, got: {:?}",
        matches
    );
}

#[test]
fn detects_todo_macro_no_message() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "fn do_thing() {\n    todo!()\n}\n";
    let matches = scanner.scan_content("src/lib.rs", content);
    assert!(
        matches.iter().any(|m| m.matched_text.contains("todo!()")),
        "Expected todo!() macro to be detected, got: {:?}",
        matches
    );
}

#[test]
fn detects_todo_macro_with_message() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "fn do_thing() {\n    todo!(\"message\")\n}\n";
    let matches = scanner.scan_content("src/lib.rs", content);
    assert!(
        matches.iter().any(|m| m.matched_text.contains("todo!")),
        "Expected todo!(\"message\") macro to be detected, got: {:?}",
        matches
    );
}

#[test]
fn detects_unimplemented_macro_no_message() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "fn do_thing() {\n    unimplemented!()\n}\n";
    let matches = scanner.scan_content("src/lib.rs", content);
    assert!(
        matches
            .iter()
            .any(|m| m.matched_text.contains("unimplemented!()")),
        "Expected unimplemented!() macro to be detected, got: {:?}",
        matches
    );
}

#[test]
fn detects_unimplemented_macro_with_message() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "fn do_thing() {\n    unimplemented!(\"msg\")\n}\n";
    let matches = scanner.scan_content("src/lib.rs", content);
    assert!(
        matches
            .iter()
            .any(|m| m.matched_text.contains("unimplemented!")),
        "Expected unimplemented!(\"msg\") macro to be detected, got: {:?}",
        matches
    );
}

#[test]
fn detects_stub_in_production_code() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "// This is a stub implementation\nfn compute() -> i32 { 0 }\n";
    let matches = scanner.scan_content("src/lib.rs", content);
    assert!(
        matches
            .iter()
            .any(|m| m.matched_text.to_lowercase().contains("stub")),
        "Expected 'stub' to be detected, got: {:?}",
        matches
    );
}

#[test]
fn detects_placeholder_in_production_code() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "// placeholder value for now\nconst X: i32 = 0;\n";
    let matches = scanner.scan_content("src/lib.rs", content);
    assert!(
        matches
            .iter()
            .any(|m| m.matched_text.to_lowercase().contains("placeholder")),
        "Expected 'placeholder' to be detected, got: {:?}",
        matches
    );
}

#[test]
fn detects_throw_not_implemented_typescript() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "function doWork() {\n    throw new Error(\"not implemented\")\n}\n";
    let matches = scanner.scan_content("src/service.ts", content);
    assert!(
        matches
            .iter()
            .any(|m| m.matched_text.contains("not implemented")),
        "Expected throw new Error(\"not implemented\") to be detected, got: {:?}",
        matches
    );
}

#[test]
fn detects_empty_function_body_rust() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "fn foo() {}\n";
    let matches = scanner.scan_content("src/lib.rs", content);
    assert!(
        !matches.is_empty(),
        "Expected empty Rust function body to be detected"
    );
}

#[test]
fn detects_empty_function_body_typescript() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "function bar() {}\n";
    let matches = scanner.scan_content("src/app.ts", content);
    assert!(
        !matches.is_empty(),
        "Expected empty TypeScript function body to be detected"
    );
}

#[test]
fn detects_pass_as_sole_function_body_python() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "def do_work():\n    pass\n";
    let matches = scanner.scan_content("src/app.py", content);
    assert!(
        matches
            .iter()
            .any(|m| m.matched_text.contains("pass")),
        "Expected Python pass-only body to be detected, got: {:?}",
        matches
    );
}

#[test]
fn detects_allow_dead_code_attribute() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "#[allow(dead_code)]\nfn unused() -> i32 { 42 }\n";
    let matches = scanner.scan_content("src/lib.rs", content);
    assert!(
        matches
            .iter()
            .any(|m| m.matched_text.contains("allow(dead_code)")),
        "Expected #[allow(dead_code)] to be detected, got: {:?}",
        matches
    );
}

// ---------------------------------------------------------------------------
// 2. Clean production file — zero matches
// ---------------------------------------------------------------------------

#[test]
fn clean_file_produces_zero_matches() {
    let scanner = ForbiddenPatternScanner::new();
    let content = r#"
/// Computes the sum of two integers.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

/// Computes the product of two integers.
pub fn multiply(a: i32, b: i32) -> i32 {
    a * b
}

pub struct Config {
    pub name: String,
    pub value: u64,
}

impl Config {
    pub fn new(name: String, value: u64) -> Self {
        Self { name, value }
    }
}
"#;
    let matches = scanner.scan_content("src/math.rs", content);
    assert!(
        matches.is_empty(),
        "Expected zero matches on clean file, got: {:?}",
        matches
    );
}

// ---------------------------------------------------------------------------
// 3. Test file exemptions
// ---------------------------------------------------------------------------

#[test]
fn test_file_detection() {
    assert!(
        ForbiddenPatternScanner::is_test_file("src/tests/foo.rs"),
        "src/tests/foo.rs should be a test file"
    );
    assert!(
        ForbiddenPatternScanner::is_test_file("src/test/bar.rs"),
        "src/test/bar.rs should be a test file"
    );
    assert!(
        ForbiddenPatternScanner::is_test_file("foo_test.rs"),
        "foo_test.rs should be a test file"
    );
    assert!(
        ForbiddenPatternScanner::is_test_file("foo.test.ts"),
        "foo.test.ts should be a test file"
    );
    assert!(
        ForbiddenPatternScanner::is_test_file("test_foo.py"),
        "test_foo.py should be a test file"
    );
    assert!(
        !ForbiddenPatternScanner::is_test_file("src/lib.rs"),
        "src/lib.rs should NOT be a test file"
    );
    assert!(
        !ForbiddenPatternScanner::is_test_file("src/main.rs"),
        "src/main.rs should NOT be a test file"
    );
}

// ---------------------------------------------------------------------------
// 4. Gate pass/fail logic
// ---------------------------------------------------------------------------

#[test]
fn gate_passed_true_when_no_matches() {
    let scanner = ForbiddenPatternScanner::new();
    let files: Vec<(&str, &str)> = vec![(
        "src/clean.rs",
        "pub fn greet(name: &str) -> String {\n    format!(\"Hello, {name}\")\n}\n",
    )];
    let result = scanner.scan_files(&files);
    assert!(
        result.gate_passed,
        "Gate should pass when there are no matches"
    );
    assert!(result.matches.is_empty());
}

#[test]
fn gate_passed_false_when_matches_found() {
    let scanner = ForbiddenPatternScanner::new();
    let files: Vec<(&str, &str)> = vec![("src/bad.rs", "fn broken() {\n    todo!()\n}\n")];
    let result = scanner.scan_files(&files);
    assert!(
        !result.gate_passed,
        "Gate should fail when forbidden patterns are found"
    );
    assert!(!result.matches.is_empty());
}

// ---------------------------------------------------------------------------
// 5. scan_files aggregation and scanned_files count
// ---------------------------------------------------------------------------

#[test]
fn scan_files_counts_and_aggregates_correctly() {
    let scanner = ForbiddenPatternScanner::new();
    let files: Vec<(&str, &str)> = vec![
        ("src/a.rs", "fn a() {\n    todo!()\n}\n"),
        (
            "src/b.rs",
            "pub fn b(x: i32) -> i32 {\n    x + 1\n}\n",
        ),
        ("src/c.rs", "fn c() {\n    // FIXME: broken\n}\n"),
    ];
    let result = scanner.scan_files(&files);
    assert_eq!(
        result.scanned_files, 3,
        "All 3 production files should be scanned"
    );
    // a.rs has todo!(), c.rs has FIXME — at least 2 matches
    assert!(
        result.matches.len() >= 2,
        "Expected at least 2 matches across files, got {}",
        result.matches.len()
    );
    // Verify matches come from different files
    let files_with_matches: std::collections::HashSet<&str> =
        result.matches.iter().map(|m| m.file.as_str()).collect();
    assert!(files_with_matches.contains("src/a.rs"));
    assert!(files_with_matches.contains("src/c.rs"));
}

// ---------------------------------------------------------------------------
// 6. Test files are skipped in scan_files
// ---------------------------------------------------------------------------

#[test]
fn scan_files_skips_test_files() {
    let scanner = ForbiddenPatternScanner::new();
    let files: Vec<(&str, &str)> = vec![
        ("src/tests/helper.rs", "fn helper() {\n    todo!()\n}\n"),
        ("foo_test.rs", "fn test_thing() {\n    unimplemented!()\n}\n"),
        ("src/lib.rs", "pub fn lib_fn() {\n    42;\n}\n"),
    ];
    let result = scanner.scan_files(&files);
    // Only src/lib.rs should be scanned; the two test files are skipped
    assert_eq!(
        result.scanned_files, 1,
        "Only non-test files should be counted as scanned"
    );
    assert!(
        result.matches.is_empty(),
        "Test files should be skipped, and the production file is clean"
    );
    assert!(result.gate_passed);
}

// ---------------------------------------------------------------------------
// 7. Line numbers are correct (1-indexed)
// ---------------------------------------------------------------------------

#[test]
fn line_numbers_are_one_indexed() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "pub fn ok() -> i32 {\n    42\n}\n// TODO: line four\nfn another() {\n    todo!()\n}\n";
    let matches = scanner.scan_content("src/lib.rs", content);

    let todo_comment = matches
        .iter()
        .find(|m| m.matched_text.contains("TODO"))
        .expect("Should detect TODO comment");
    assert_eq!(
        todo_comment.line, 4,
        "TODO comment is on line 4 (1-indexed)"
    );

    let todo_macro = matches
        .iter()
        .find(|m| m.matched_text.contains("todo!()"))
        .expect("Should detect todo!() macro");
    assert_eq!(
        todo_macro.line, 6,
        "todo!() macro is on line 6 (1-indexed)"
    );
}

// ---------------------------------------------------------------------------
// 8. Pattern names identify which pattern was triggered
// ---------------------------------------------------------------------------

#[test]
fn pattern_names_are_descriptive() {
    let scanner = ForbiddenPatternScanner::new();
    let content = "// TODO: something\nfn x() {\n    unimplemented!()\n}\n";
    let matches = scanner.scan_content("src/lib.rs", content);

    let todo_match = matches
        .iter()
        .find(|m| m.matched_text.contains("TODO"))
        .expect("Should find TODO match");
    assert!(
        !todo_match.pattern_name.is_empty(),
        "Pattern name should not be empty"
    );

    let unimpl_match = matches
        .iter()
        .find(|m| m.matched_text.contains("unimplemented!"))
        .expect("Should find unimplemented! match");
    assert!(
        !unimpl_match.pattern_name.is_empty(),
        "Pattern name should not be empty"
    );

    // The two different patterns should have different names
    assert_ne!(
        todo_match.pattern_name, unimpl_match.pattern_name,
        "Different patterns should have different names"
    );
}

// ---------------------------------------------------------------------------
// Comprehensive: all forbidden patterns in a single file
// ---------------------------------------------------------------------------

#[test]
fn all_forbidden_patterns_detected_in_single_file() {
    let scanner = ForbiddenPatternScanner::new();
    // A maximally bad Rust file containing every forbidden pattern
    let content = r#"#[allow(dead_code)]
// TODO: fix this
// FIXME: broken
// HACK: workaround
// XXX: dangerous
fn empty_body() {}
fn with_todo() {
    todo!()
}
fn with_todo_msg() {
    todo!("message")
}
fn with_unimplemented() {
    unimplemented!()
}
fn with_unimplemented_msg() {
    unimplemented!("msg")
}
// stub implementation
// placeholder value
"#;
    let matches = scanner.scan_content("src/terrible.rs", content);
    // We expect at least one match for each of the Rust-applicable patterns:
    // TODO, FIXME, HACK, XXX, todo!(), todo!("message"), unimplemented!(),
    // unimplemented!("msg"), stub, placeholder, empty function body, allow(dead_code)
    let pattern_names: Vec<&str> = matches.iter().map(|m| m.pattern_name.as_str()).collect();
    assert!(
        matches.len() >= 12,
        "Expected at least 12 matches for all forbidden patterns, got {} with patterns: {:?}",
        matches.len(),
        pattern_names
    );
}
