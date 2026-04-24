// TASK-P0-B.5 (#183): ApplyPatch — hunk applier.
//
// Applies a sequence of parsed hunks against the original file
// contents, verifying every context and removed line.

use super::parser::{Hunk, HunkLine};

/// Apply a sequence of parsed hunks against the original file contents.
///
/// The result preserves the original file's trailing-newline disposition:
/// if the input ended with `\n`, so does the output; if it did not, the
/// output also has no trailing `\n`. This mirrors `diff -u` semantics
/// and keeps hunk editing non-destructive at the file boundary.
pub(super) fn apply_hunks(original: &str, hunks: &[Hunk]) -> Result<String, String> {
    // Split retaining the line terminator so we can distinguish a
    // trailing newline from its absence and still reconstruct exactly.
    let original_had_trailing_newline = original.ends_with('\n');
    let original_lines: Vec<&str> = if original.is_empty() {
        Vec::new()
    } else {
        original.lines().collect()
    };

    // Build the result as a Vec<String> of logical lines (no terminators).
    let mut out: Vec<String> = Vec::new();
    let mut cursor: usize = 0; // index into original_lines (0-based)

    for (idx, hunk) in hunks.iter().enumerate() {
        // Header `old_start` is 1-based; a start of 0 is only valid for
        // the empty-file case where old_len == 0.
        let hunk_start_idx = if hunk.old_start == 0 {
            if hunk.old_len != 0 {
                return Err(format!(
                    "hunk {} has old_start=0 but old_len={} (must be 0)",
                    idx + 1,
                    hunk.old_len
                ));
            }
            0
        } else {
            hunk.old_start - 1
        };

        if hunk_start_idx < cursor {
            return Err(format!(
                "hunk {} starts at line {} but previous hunk already consumed up to line {}",
                idx + 1,
                hunk.old_start,
                cursor
            ));
        }

        // Copy unchanged lines between the previous hunk and this one.
        while cursor < hunk_start_idx {
            if cursor >= original_lines.len() {
                return Err(format!(
                    "hunk {} references line {} but file only has {} lines",
                    idx + 1,
                    hunk.old_start,
                    original_lines.len()
                ));
            }
            out.push(original_lines[cursor].to_string());
            cursor += 1;
        }

        // Walk the hunk body, verifying context/remove against original
        // and emitting context/add into the output.
        let mut consumed_old = 0usize;
        for hline in &hunk.lines {
            match hline {
                HunkLine::Context(expected) => {
                    if cursor >= original_lines.len() {
                        return Err(format!(
                            "hunk {} expected context line {:?} but file ended at line {}",
                            idx + 1,
                            expected,
                            cursor
                        ));
                    }
                    let actual = original_lines[cursor];
                    if actual != expected {
                        return Err(format!(
                            "hunk {} context mismatch at line {}: expected {:?}, found {:?}",
                            idx + 1,
                            cursor + 1,
                            expected,
                            actual
                        ));
                    }
                    out.push(actual.to_string());
                    cursor += 1;
                    consumed_old += 1;
                }
                HunkLine::Remove(expected) => {
                    if cursor >= original_lines.len() {
                        return Err(format!(
                            "hunk {} expected to remove line {:?} but file ended at line {}",
                            idx + 1,
                            expected,
                            cursor
                        ));
                    }
                    let actual = original_lines[cursor];
                    if actual != expected {
                        return Err(format!(
                            "hunk {} remove mismatch at line {}: expected {:?}, found {:?}",
                            idx + 1,
                            cursor + 1,
                            expected,
                            actual
                        ));
                    }
                    cursor += 1;
                    consumed_old += 1;
                }
                HunkLine::Add(added) => {
                    out.push(added.clone());
                }
            }
        }

        if consumed_old != hunk.old_len {
            return Err(format!(
                "hunk {} header declares old_len={} but body consumed {} old lines",
                idx + 1,
                hunk.old_len,
                consumed_old
            ));
        }
    }

    // Copy any trailing lines after the last hunk verbatim.
    while cursor < original_lines.len() {
        out.push(original_lines[cursor].to_string());
        cursor += 1;
    }

    // Reassemble. Every line joined with '\n'; preserve the original
    // trailing-newline disposition on the last line.
    let mut result = out.join("\n");
    if original_had_trailing_newline && !out.is_empty() {
        result.push('\n');
    }
    // Edge: original was empty, patch added lines — always end with \n
    // so the file is a well-formed POSIX text file.
    if original.is_empty() && !out.is_empty() {
        result.push('\n');
    }
    Ok(result)
}
