//! Fenced-block awareness shared by the prose lints (Issue-61).
//!
//! Four lints read a Markdown body line by line and skip fenced code: the
//! deliverable observations, the repository claims, the PRD's path literals
//! for owner coverage, and the focused-test declarations. Each carried its
//! own `fenced = !fenced` toggle. The toggle is right for a well-formed
//! document and wrong in the same way for all four on one that is not: a
//! body author who wraps the whole reply in an outer ```` ```markdown ````
//! fence (observed on a live run) inverts the state at the first
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

/// `text` without a leading UTF-8 byte order mark and without its leading
/// blank lines (whitespace only, ended by `\n` or `\r\n`). The rest is the
/// same bytes.
pub(crate) fn strip_leading_blank_lines(text: &str) -> &str {
    let mut rest = text.strip_prefix('\u{feff}').unwrap_or(text);
    while let Some(end) = rest.find('\n') {
        if !rest[..end].trim().is_empty() {
            break;
        }
        rest = &rest[end + 1..];
    }
    rest
}

/// The first line of `text` after [`strip_leading_blank_lines`], without its
/// line terminator.
pub(crate) fn first_nonblank_line(text: &str) -> &str {
    let rest = strip_leading_blank_lines(text);
    let line = rest.split('\n').next().unwrap_or_default();
    line.strip_suffix('\r').unwrap_or(line)
}

/// The task file's own frontmatter opener: ```` ```yaml ```` or
/// ```` ```yml ````, with surrounding whitespace allowed (the parser reads
/// the line trimmed).
pub(crate) fn is_frontmatter_opener(line: &str) -> bool {
    matches!(line.trim(), "```yaml" | "```yml")
}

/// How a task file's first non-blank line places its frontmatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskFileShape<'a> {
    /// The frontmatter opener is the first non-blank line.
    Frontmatter,
    /// A pure whole-document fence ([`unwrap_outer_fence`]); the interior.
    Wrapped(&'a str),
    /// Anything else: the first non-blank line, which is not the frontmatter.
    TextBefore(&'a str),
}

pub(crate) fn task_file_shape(text: &str) -> TaskFileShape<'_> {
    let first = first_nonblank_line(text);
    if is_frontmatter_opener(first) {
        return TaskFileShape::Frontmatter;
    }
    match unwrap_outer_fence(strip_leading_blank_lines(text)) {
        Some(interior) => TaskFileShape::Wrapped(interior),
        None => TaskFileShape::TextBefore(first),
    }
}

/// `line` trimmed and cut to at most 120 characters, for a finding.
pub(crate) fn quoted_first_line(line: &str) -> String {
    line.trim().chars().take(120).collect()
}

/// The one finding for a task file whose first non-blank line is not its
/// frontmatter.
pub(crate) fn text_before_frontmatter_finding(first_line: &str) -> String {
    format!(
        "the task file starts with text before its frontmatter (first line: \"{}\"); return only the task file, starting with its ```yaml frontmatter block",
        quoted_first_line(first_line)
    )
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

/// A task file found after leading packaging in an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Packaged<'a> {
    /// The text the host discards before the task file, a wrapper's opener
    /// included.
    pub(crate) leading: &'a str,
    /// The task file, its bytes as written.
    pub(crate) task_file: &'a str,
    /// Whether a wrapper fence pair was removed (its closer too).
    pub(crate) wrapper: bool,
}

/// The task file in `text` after leading packaging, or `None`.
///
/// The task file opens at the first ```` ```yaml ```` line whose block parses
/// as a mapping with a `task_id`. When the last non-blank line before it is a
/// wrapper opener ([`unwrap_outer_fence`] rule 1) outside any fenced block,
/// the task file opens there instead, and the text from that wrapper on must
/// be a pure outer fence, whose interior is the task file. Every line before
/// the opener is packaging; nothing after the task file is touched except
/// that wrapper's closer. Chat that leaves a block open before the opener is
/// `None`, for the caller to refuse. Whether the rest holds one task file is
/// [`task_files`]'s question, asked of every candidate.
pub(crate) fn strip_packaging(text: &str) -> Option<Packaged<'_>> {
    let spans: Vec<_> = line_spans(text).collect();
    let opener = frontmatter_task_ids(text, &spans)
        .iter()
        .position(Option::is_some)?;
    let wrapper = spans[..opener]
        .iter()
        .rposition(|(_, _, line)| !line.trim().is_empty())
        .filter(|&index| is_outer_opener(spans[index].2) && !fenced_after(&spans[..index]));
    let (leading, task_file) = match wrapper {
        Some(index) => {
            let interior = unwrap_outer_fence(&text[spans[index].0..])?;
            let task_file = strip_leading_blank_lines(interior);
            (&text[..offset_in(text, task_file)], task_file)
        }
        // Chat that leaves a block open would put the task file inside it.
        None if fenced_after(&spans[..opener]) => return None,
        None => text.split_at(spans[opener].0),
    };
    (!leading.is_empty()).then_some(Packaged {
        leading,
        task_file,
        wrapper: wrapper.is_some(),
    })
}

/// The byte offset of `part`, a slice of `text`, within it.
pub(crate) fn offset_in(text: &str, part: &str) -> usize {
    part.as_ptr() as usize - text.as_ptr() as usize
}

/// The task files in `task_file`, by `task_id`: its own frontmatter (the
/// first top-level ```` ```yaml ```` block, `None` when it names no
/// `task_id`), then every later top-level block that opens another whole
/// task file. A later block counts only when the task parser accepts the
/// text from it to the next such block as a task file under its own id, so
/// a yaml example in the body, even one carrying a `task_id`, never does.
pub(crate) fn task_files(task_file: &str) -> (Option<String>, Vec<String>) {
    let spans: Vec<_> = line_spans(task_file).collect();
    let ids = frontmatter_task_ids(task_file, &spans);
    let mut fenced = false;
    let mut openers = Vec::new();
    for (index, (_, _, line)) in spans.iter().enumerate() {
        if is_fence_line(line) {
            if !fenced && is_frontmatter_opener(line) {
                openers.push(index);
            }
            fenced = !fenced;
        }
    }
    let Some((&first, later)) = openers.split_first() else {
        return (None, Vec::new());
    };
    let named: Vec<usize> = later
        .iter()
        .copied()
        .filter(|&i| ids[i].is_some())
        .collect();
    let others = named
        .iter()
        .enumerate()
        .filter_map(|(slot, &index)| {
            let id = ids[index].as_deref()?;
            let end = named
                .get(slot + 1)
                .map_or(task_file.len(), |&next| spans[next].0);
            let path = std::path::PathBuf::from(format!("{id}.md"));
            let raw = &task_file[spans[index].0..end];
            archon_workflow::task_universe::parsing::parse_task_file(&path, raw)
                .is_ok()
                .then(|| id.to_string())
        })
        .collect();
    (ids[first].clone(), others)
}

/// For each line of `text`, the `task_id` of the ```` ```yaml ```` block it
/// opens, closed as the task parser closes it (a bare ```` ``` ```` or
/// `---`), when that block parses as a mapping with a string `task_id`. One
/// backward pass finds each line's closer, so the cost stays linear.
fn frontmatter_task_ids(text: &str, spans: &[(usize, usize, &str)]) -> Vec<Option<String>> {
    let mut ids = vec![None; spans.len()];
    let mut closer = None;
    for index in (0..spans.len()).rev() {
        let (start, _, line) = spans[index];
        if is_frontmatter_opener(line)
            && let Some(end) = closer
        {
            let body_start = spans.get(index + 1).map_or(text.len(), |span| span.0);
            ids[index] = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&text[body_start..end])
                .ok()
                .and_then(|value| value.get("task_id")?.as_str().map(str::to_string));
        }
        if matches!(line.trim(), "```" | "---") {
            closer = Some(start);
        }
    }
    ids
}

/// Whether a fenced block is still open after `spans`, under the shared
/// toggle.
fn fenced_after(spans: &[(usize, usize, &str)]) -> bool {
    spans
        .iter()
        .filter(|(_, _, line)| is_fence_line(line))
        .count()
        % 2
        == 1
}

#[cfg(test)]
#[path = "fences_tests.rs"]
pub(super) mod fences_tests;
