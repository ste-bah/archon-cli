//! Lifting fenced code blocks out of model output.
//!
//! Models wrap JSON and YAML in a ```` ```json ```` fence even when the prompt
//! says not to, sometimes behind a "Here you go:" preamble and sometimes with
//! commentary after the block. Five call sites across the workspace grew their
//! own stripper after being bitten by that, and they disagreed on every edge:
//! whether a preamble was tolerated, whether a bare ```` ``` ```` fence counted,
//! what an unterminated fence meant. This module is the single answer.
//!
//! # Why this crate owns it
//!
//! `archon-core::schema_validation` had the best of the five, but it cannot be
//! the home: `archon-core` depends on `archon-memory`, and `archon-memory` is
//! one of the callers, so the edge would close a cycle. `archon-context` has no
//! `archon-*` dependencies at all — it is a graph leaf, so no caller can ever
//! create a cycle by reaching it — its third-party dependencies are already in
//! every caller's tree, and manipulating conversation text is what it is for.
//!
//! # Where this is stricter than the code it replaces
//!
//! Fences are matched at the start of a line, per CommonMark, rather than
//! anywhere in the text. All five of the originals used a bare `find("```")`,
//! which truncates a block the moment a body line quotes a triple backtick
//! inside a string — a real shape when the model is summarising markdown.

/// One fenced code block, borrowed out of the text it was found in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FencedBlock<'a> {
    /// The language word of the opening fence's info string (`json`, `yaml`),
    /// or `None` for a bare ```` ``` ```` fence. Only the first word: a model
    /// that writes ```` ```json (as requested) ```` still reports `json`.
    pub tag: Option<&'a str>,
    /// The block body, verbatim — no trimming, because a caller that reads it
    /// line by line needs exactly the lines the model wrote.
    pub body: &'a str,
    /// `false` when the text ran out before a closing fence. The body is then
    /// everything after the opening fence: a truncated response still usually
    /// carries the payload, and returning nothing would throw it away.
    pub terminated: bool,
}

/// The first fenced block in `text`, whatever its language tag.
pub fn first_fenced_block(text: &str) -> Option<FencedBlock<'_>> {
    find_block(text, None)
}

/// The first fenced block whose language tag is `tag`, compared
/// case-insensitively.
///
/// Blocks with other tags are skipped rather than ending the search, so a
/// document that opens with a ```` ```bash ```` example still yields its
/// ```` ```yaml ```` metadata.
pub fn fenced_block_tagged<'a>(text: &'a str, tag: &str) -> Option<FencedBlock<'a>> {
    find_block(text, Some(tag))
}

/// Pull the JSON payload out of a model response.
///
/// Unwraps a fenced block when there is one — with or without a language tag,
/// with or without a preamble or trailing commentary — then falls back to the
/// outermost `{...}` / `[...]` span when the model answered in prose with the
/// JSON embedded in it. Text containing neither comes back trimmed and
/// otherwise untouched: the KB compiler keeps such a response as the document
/// summary rather than discarding it, so this must not return an empty string
/// for prose.
pub fn json_payload(text: &str) -> &str {
    let trimmed = text.trim();

    if let Some(block) = first_fenced_block(trimmed) {
        let body = block.body.trim();
        // An empty body means the opening fence had no block under it — a
        // single-line ```` ```json {...}``` ```` span, say, which CommonMark
        // treats as inline code. Fall through to the bracket scan, which still
        // recovers the value, rather than reporting nothing.
        if !body.is_empty() {
            return body;
        }
    }

    // Unfenced but wrapped in prose: take the outermost JSON value.
    if !trimmed.starts_with(['{', '[']) {
        let open = trimmed.find(['{', '[']);
        let close = trimmed.rfind(['}', ']']);
        if let (Some(open), Some(close)) = (open, close)
            && close > open
        {
            return trimmed[open..=close].trim();
        }
    }

    trimmed
}

/// Scan forward for a fenced block, optionally requiring a language tag.
fn find_block<'a>(text: &'a str, want: Option<&str>) -> Option<FencedBlock<'a>> {
    let mut cursor = 0;
    while cursor < text.len() {
        let (line, next) = split_line(text, cursor);
        let Some(width) = fence_width(line) else {
            cursor = next;
            continue;
        };

        let tag = info_tag(line, width);
        let (body_end, after_close, terminated) = find_close(text, next, width);
        let matches = want.is_none_or(|want| tag.is_some_and(|tag| tag.eq_ignore_ascii_case(want)));
        if matches {
            return Some(FencedBlock {
                tag,
                body: &text[next..body_end],
                terminated,
            });
        }

        // Resume after the whole block, never inside it: a body line that looks
        // like a fence belongs to this block, not to the next one.
        cursor = after_close;
    }
    None
}

/// `(body_end, resume_from, terminated)` for a block opened with `width`
/// backticks whose body starts at `from`.
fn find_close(text: &str, from: usize, width: usize) -> (usize, usize, bool) {
    let mut cursor = from;
    while cursor < text.len() {
        let (line, next) = split_line(text, cursor);
        if is_closing_fence(line, width) {
            return (cursor, next, true);
        }
        cursor = next;
    }
    (text.len(), text.len(), false)
}

/// The line starting at `at` (without its terminator) and the offset of the
/// next line.
fn split_line(text: &str, at: usize) -> (&str, usize) {
    match text[at..].find('\n') {
        Some(offset) => (
            text[at..at + offset].trim_end_matches('\r'),
            at + offset + 1,
        ),
        None => (&text[at..], text.len()),
    }
}

/// How many backticks open a fence on this line, if it opens one at all.
fn fence_width(line: &str) -> Option<usize> {
    // CommonMark allows up to three spaces of indent before a fence; beyond
    // that the line is an indented code block and not a fence.
    let indented = line.trim_start_matches(' ');
    if line.len() - indented.len() > 3 {
        return None;
    }
    let width = indented.bytes().take_while(|byte| *byte == b'`').count();
    (width >= 3).then_some(width)
}

/// The language word following the opening backticks, if there is one.
fn info_tag(line: &str, width: usize) -> Option<&str> {
    line.trim_start_matches(' ')[width..]
        .split_whitespace()
        .next()
}

/// A closing fence is at least as long as its opener and carries nothing else.
fn is_closing_fence(line: &str, open_width: usize) -> bool {
    // Requiring the rest of the line to be empty is what stops a body line from
    // closing the block early — both a nested ```` ```json ```` opener and, far
    // more common, a triple backtick quoted inside a string value, which never
    // sits alone on its own line.
    match fence_width(line) {
        Some(width) => {
            width >= open_width && line.trim_start_matches(' ')[width..].trim().is_empty()
        }
        None => false,
    }
}
