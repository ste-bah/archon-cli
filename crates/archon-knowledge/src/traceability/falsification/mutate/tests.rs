use super::*;

#[test]
fn the_anchored_range_is_replaced_and_nothing_else_moves() {
    let original = "a\nb\nc\nd\n";
    let mutant = render_mutant(original, 2, 3, "BOOM").expect("mutant");
    assert_eq!(mutant, "a\nBOOM\nd\n");
}

#[test]
fn a_crlf_file_stays_crlf_outside_the_hunk() {
    let original = "a\r\nb\r\nc\r\n";
    let mutant = render_mutant(original, 2, 2, "BOOM").expect("mutant");
    // Byte-for-byte outside the replaced line, including the line the
    // replacement took over: a mutant that rewrote every line ending would be
    // testing the rewrite, not the anchor.
    assert_eq!(mutant, "a\r\nBOOM\r\nc\r\n");
}

#[test]
fn the_replacement_inherits_the_indentation_of_the_first_replaced_line() {
    let original = "def f():\n    x = 1\n    return x\n";
    let mutant = render_mutant(original, 2, 3, "raise AssertionError('probe')").expect("mutant");
    assert_eq!(mutant, "def f():\n    raise AssertionError('probe')\n");
}

#[test]
fn a_file_with_no_trailing_newline_keeps_not_having_one() {
    let original = "a\nb";
    assert_eq!(
        render_mutant(original, 2, 2, "BOOM").expect("mutant"),
        "a\nBOOM"
    );
}

#[test]
fn a_range_past_the_end_of_the_file_is_refused_rather_than_clamped() {
    let err = render_mutant("a\nb\n", 2, 9, "BOOM").expect_err("refused");
    assert_eq!(
        err,
        MutationError::RangeOutOfFile {
            line_end: 9,
            line_count: 2
        }
    );
    assert!(
        err.describe().contains("different file"),
        "{}",
        err.describe()
    );
}

#[test]
fn a_degenerate_range_is_refused() {
    assert_eq!(
        render_mutant("a\n", 0, 1, "BOOM"),
        Err(MutationError::DegenerateRange {
            line_start: 0,
            line_end: 1
        })
    );
    assert!(render_mutant("a\nb\n", 2, 1, "BOOM").is_err());
}

#[test]
fn a_language_is_derived_from_the_extension_or_not_at_all() {
    assert_eq!(language_for_path("src/a.rs"), Some("rust"));
    assert_eq!(language_for_path("pkg/mod.go"), Some("go"));
    assert_eq!(language_for_path("web/app.tsx"), Some("typescript"));
    assert_eq!(language_for_path("s/x.py"), Some("python"));
    // No guess for an extension nobody listed, and none for a file without one.
    assert_eq!(language_for_path("Makefile"), None);
    assert_eq!(language_for_path("src/a.cobol"), None);
}
