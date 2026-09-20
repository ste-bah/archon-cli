//! Fenced-block awareness shared by the prose lints (Issue-61).
//!
//! Four lints read a Markdown body line by line and skip fenced code: the
//! deliverable observations, the repository claims, the PRD's path literals
//! for owner coverage, and the focused-test declarations. Each carried its
//! own `fenced = !fenced` toggle. The toggle is right for a well-formed
//! document and wrong in the same way for all four on one that is not: a
//! body author who wraps the whole reply in an outer ```` ```markdown ````
//! fence (run wf-7acf8d4a, TASK-TRADING-002) inverts the state at the first
//! line, so once the frontmatter closes everything to the end of the file
//! reads as fenced and every correct observation is invisible. The lints then
//! send the body back for a defect it does not have, attempt after attempt.
//!
//! This module owns the two answers. [`prose_lines`] is the one toggle the
//! lints share, so they cannot drift apart again; it still reads a wrapped
//! document wrongly, which its tests pin. [`unwrap_outer_fence`] is
//! the repair, applied once at the body gate's entry before any lint runs and
//! before the candidate is staged, so the landed file is the document the
//! author meant and every reader downstream sees the same text.

/// A line that opens or closes a fenced block: three backticks after any
/// indentation, with or without an info string.
pub(crate) fn is_fence_line(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

/// How one line of a document reads to a prose lint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineKind {
    /// Outside every fenced block: prose the lints read.
    Prose,
    /// A fence line itself; never prose.
    Fence,
    /// Inside a fenced block; never prose.
    Fenced,
}

/// Every line of `text` with its kind, under the lints' shared toggle: a
/// fence line flips the state, whatever its info string.
pub(crate) fn classified_lines(text: &str) -> impl Iterator<Item = (LineKind, &str)> {
    let mut fenced = false;
    text.lines().map(move |line| {
        if is_fence_line(line) {
            fenced = !fenced;
            (LineKind::Fence, line)
        } else if fenced {
            (LineKind::Fenced, line)
        } else {
            (LineKind::Prose, line)
        }
    })
}

/// The prose lines of `text`: fence lines and fenced blocks are not prose.
pub(crate) fn prose_lines(text: &str) -> impl Iterator<Item = &str> {
    classified_lines(text)
        .filter(|(kind, _)| *kind == LineKind::Prose)
        .map(|(_, line)| line)
}

/// The document inside a whole-document code fence, or `None` when `text` is
/// not wrapped in one. The rule, all parts required:
///
/// 1. the first non-blank line is a fence opener whose info string is empty
///    or a single word that is not `yaml`/`yml` (`markdown`, `md`, …);
/// 2. the last non-blank line is a bare closing ```` ``` ````;
/// 3. the interior between them opens with the task's own ```` ```yaml ````
///    (or ```` ```yml ````) frontmatter as its first non-blank line;
/// 4. the interior's fence lines pair off (an even count), so the outer pair
///    is surplus and not one half of an inner block.
///
/// The interior is returned as written, from the start of the line after the
/// opener to the end of the line before the closer, its own line endings
/// intact. Nothing else is touched: a document that legitimately starts with
/// ```` ```yaml ```` fails rule 1 and is returned unchanged by the caller.
pub(crate) fn unwrap_outer_fence(text: &str) -> Option<&str> {
    let mut lines = line_spans(text).filter(|(_, _, line)| !line.trim().is_empty());
    let (opener_start, opener_end, opener) = lines.next()?;
    if !is_outer_opener(opener) {
        return None;
    }
    let (closer_start, _, closer) = lines.last()?;
    if closer.trim() != "```" || closer_start <= opener_start {
        return None;
    }
    let interior = &text[after_line_end(text, opener_end)..closer_start];
    let mut interior_lines = interior.lines().filter(|line| !line.trim().is_empty());
    if !interior_lines
        .next()
        .is_some_and(|line| matches!(line.trim(), "```yaml" | "```yml"))
    {
        return None;
    }
    let fence_lines = interior.lines().filter(|line| is_fence_line(line)).count();
    (fence_lines % 2 == 0).then_some(interior)
}

/// A fence opener that can only be wrapping the document: bare, or carrying
/// one info word that is not the frontmatter's own `yaml`/`yml`.
fn is_outer_opener(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(info) = trimmed.strip_prefix("```") else {
        return false;
    };
    let info = info.trim_start_matches('`').trim();
    info.is_empty()
        || (!info.contains(char::is_whitespace)
            && !info.eq_ignore_ascii_case("yaml")
            && !info.eq_ignore_ascii_case("yml"))
}

/// Each line of `text` as `(start, end, line)` byte offsets, `end` excluding
/// the line terminator.
fn line_spans(text: &str) -> impl Iterator<Item = (usize, usize, &str)> {
    let mut offset = 0;
    text.split_inclusive('\n').map(move |raw| {
        let start = offset;
        offset += raw.len();
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let line = line.strip_suffix('\r').unwrap_or(line);
        (start, start + line.len(), line)
    })
}

/// The offset just past the terminator of the line whose content ends at
/// `line_end`.
fn after_line_end(text: &str, line_end: usize) -> usize {
    let rest = &text[line_end..];
    let terminator = if rest.starts_with("\r\n") {
        2
    } else if rest.starts_with('\n') {
        1
    } else {
        0
    };
    line_end + terminator
}

#[cfg(test)]
#[path = "fences_tests.rs"]
pub(super) mod fences_tests;
