//! Rendering the mutant, as a pure text transform.
//!
//! Nothing here opens, writes or names a file on disk. The mutation is
//! `original text + line range + replacement` → `new text`, so *what the mutant
//! looks like* stays in this crate beside the rest of the ladder, while *the
//! decision to write it into someone's working tree* stays with the opt-in
//! executor in the command layer. Splitting it that way is what lets the whole
//! mutation be tested without a tempdir, a process, or a repository.

use serde::{Deserialize, Serialize};

/// The language an anchored file is written in, derived from its extension.
///
/// [`super::super::anchors::Anchor`] records no language. It could — the index
/// hit carries one — but a language field on a recorded edge is a second fact
/// about the file that can drift from the file, and the extension cannot. So
/// this is derived at mutation time from the one property of the path that is
/// part of the path.
///
/// An unrecognised extension returns `None`, and the executor refuses rather
/// than guessing an abort form, exactly as [`super::MutationKind::replacement_for`]
/// does for an unknown language.
pub fn language_for_path(path: &str) -> Option<&'static str> {
    let (_, ext) = path.rsplit_once('.')?;
    Some(match ext {
        "rs" => "rust",
        "py" => "python",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "go" => "go",
        _ => return None,
    })
}

/// Why a line range could not be replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationError {
    /// `line_start` is 0, or `line_end` precedes it. Anchors are 1-based and
    /// inclusive; a range that is neither describes no code.
    DegenerateRange { line_start: usize, line_end: usize },
    /// The range runs past the end of the file. The anchor was recorded against
    /// a file with more lines than this one has, which means the plan is stale
    /// even if the hash check somehow passed.
    RangeOutOfFile { line_end: usize, line_count: usize },
}

impl MutationError {
    pub fn describe(&self) -> String {
        match self {
            MutationError::DegenerateRange {
                line_start,
                line_end,
            } => format!(
                "anchored range {line_start}-{line_end} is not a 1-based inclusive range; \
                 there is nothing to replace"
            ),
            MutationError::RangeOutOfFile {
                line_end,
                line_count,
            } => format!(
                "anchored range ends at line {line_end} but the file has {line_count} line(s); \
                 the plan was written against a different file"
            ),
        }
    }
}

/// Replace lines `line_start..=line_end` (1-based, inclusive) with `replacement`.
///
/// Two properties this preserves deliberately, because a mutant that differs
/// from the original anywhere outside the anchored range is a mutant that tests
/// something other than the anchor:
///
/// 1. **Line endings.** Splitting on `'\n'` rather than by [`str::lines`] keeps
///    any `\r` attached to each line, so a CRLF file round-trips byte-for-byte
///    outside the replaced hunk instead of being silently rewritten to LF.
/// 2. **Indentation.** The replacement inherits the leading whitespace of the
///    first replaced line. For Rust that is cosmetic; for Python it is the
///    difference between an abort and an `IndentationError`, and an
///    `IndentationError` proves nothing about the verifier.
pub fn render_mutant(
    original: &str,
    line_start: usize,
    line_end: usize,
    replacement: &str,
) -> std::result::Result<String, MutationError> {
    if line_start == 0 || line_end < line_start {
        return Err(MutationError::DegenerateRange {
            line_start,
            line_end,
        });
    }
    let parts: Vec<&str> = original.split('\n').collect();
    // A file ending in a newline splits into a final empty piece that is not a
    // line. Counting it would let a range one past the end look in-bounds.
    let line_count = if parts.last() == Some(&"") {
        parts.len() - 1
    } else {
        parts.len()
    };
    if line_end > line_count {
        return Err(MutationError::RangeOutOfFile {
            line_end,
            line_count,
        });
    }

    let indent: String = parts[line_start - 1]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let carriage = parts[line_end - 1].ends_with('\r');

    let mut out = String::with_capacity(original.len());
    for (idx, part) in parts.iter().enumerate() {
        let line_no = idx + 1;
        if line_no > line_start && line_no <= line_end {
            // Swallowed by the replacement, newline and all.
            continue;
        }
        if line_no == line_start {
            out.push_str(&indent);
            out.push_str(replacement);
            if carriage {
                out.push('\r');
            }
        } else {
            out.push_str(part);
        }
        if idx + 1 < parts.len() {
            out.push('\n');
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
