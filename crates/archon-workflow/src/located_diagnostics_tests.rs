use super::*;
#[test]
fn located_error_formats_keep_paths_and_rows() {
    for (output, path, row, output_line) in [
        ("./a.go:5:2: undefined: f", "./a.go", 5, 0),
        (
            "[ERROR] /x/A.java:[5,10] cannot find symbol",
            "/x/A.java",
            5,
            0,
        ),
        ("a.ts(3,5): error TS123: bad", "a.ts", 3, 0),
        ("a.ts:3:5 - error TS123: bad", "a.ts", 3, 0),
        ("error[E123]: bad\n --> src/a.rs:9:2", "src/a.rs", 9, 1),
        ("File \"x.py\", line 2", "x.py", 2, 0),
        ("C:\\r\\a.go:5:2: bad\r\n", "C:\\r\\a.go", 5, 0),
    ] {
        let found = located_lines(output);
        assert_eq!(
            found,
            vec![Located {
                path: path.into(),
                line: row,
                output_line,
                kind: Kind::Error
            }],
            "{output}"
        );
    }
}
#[test]
fn runtime_frames_timestamps_and_unlocated_text_are_not_compile_locations() {
    for output in [
        "File \"x.py\", line 2, in f",
        "12:30:45 run",
        "AssertionError: mismatch",
    ] {
        assert!(located_lines(output).is_empty(), "{output}");
    }
}
#[test]
fn warning_note_and_help_locations_are_not_error_locations() {
    for output in [
        "a.ts:3:5: warning: bad",
        "a.ts:3:5: NOTE: note",
        "help: text\n --> a.ts:3:5",
    ] {
        assert!(
            located_lines(output)
                .iter()
                .all(|line| line.kind == Kind::Other),
            "{output}"
        );
    }
}
