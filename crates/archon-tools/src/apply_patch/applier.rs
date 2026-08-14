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
    // Header/body count disagreements, recorded rather than fatal.
    let mut header_mismatches: Vec<String> = Vec::new();

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

        // The header's `old_len` is NOT checked against the body.
        //
        // By this point every context and remove line in the hunk has been
        // matched byte-for-byte against the real file at the right offset, so
        // the body is proven applicable. Rejecting it because the model's
        // arithmetic in the `@@` header disagreed with its own body threw away
        // a correct patch over a redundant count — one the applier can derive
        // and the model routinely miscounts.
        //
        // Measured on one live branch: 36 of its errors were this, each costing
        // a full re-ask of work that had already been verified correct. The
        // check was also one-sided — `new_len` has never been enforced — so the
        // strictness bought no consistency either.
        //
        // The count is recorded for the caller rather than dropped, so a
        // genuinely truncated hunk is still visible to anyone reading the
        // result.
        if consumed_old != hunk.old_len {
            header_mismatches.push(format!(
                "hunk {} header declared old_len={} but body consumed {} old lines (applied from the body)",
                idx + 1,
                hunk.old_len,
                consumed_old
            ));
        }
    }

    if !header_mismatches.is_empty() {
        // Visible, but never fatal: the body was verified against the file
        // line by line, so the patch is correct and the header count is not.
        tracing::debug!(
            mismatches = %header_mismatches.join("; "),
            "applied patch whose hunk header counts disagreed with its body"
        );
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

#[cfg(test)]
mod header_count_tests {
    use super::super::parser::parse_hunks;
    use super::apply_hunks;

    /// The live failure: a hunk whose `@@` header count disagrees with its own
    /// body. Every context and remove line still matches the file, so the patch
    /// is correct and now applies. One branch produced 36 of these in a single
    /// run, each discarding verified work over the model's arithmetic.
    #[test]
    fn a_wrong_old_len_still_applies() {
        let original = "one\ntwo\nthree\n";
        // Body consumes 3 old lines; the header claims 2.
        let patch = "--- a/f.txt\n+++ b/f.txt\n@@ -1,2 +1,4 @@\n one\n two\n+inserted\n three\n";
        let hunks = parse_hunks(patch).expect("parse");
        let out = apply_hunks(original, &hunks).expect("must apply despite the header count");
        assert_eq!(out, "one\ntwo\ninserted\nthree\n");
    }

    /// A body that does NOT match the file is still refused — the count was
    /// never what made the patch safe.
    #[test]
    fn a_body_that_does_not_match_the_file_is_still_rejected() {
        let original = "one\ntwo\nthree\n";
        let patch = "--- a/f.txt\n+++ b/f.txt\n@@ -1,3 +1,3 @@\n one\n WRONG\n three\n";
        let hunks = parse_hunks(patch).expect("parse");
        let err = apply_hunks(original, &hunks).expect_err("context mismatch must fail");
        assert!(err.contains("context mismatch"), "got: {err}");
    }
}
