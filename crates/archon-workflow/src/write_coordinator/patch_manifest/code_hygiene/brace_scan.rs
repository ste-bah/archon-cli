//! Function spans in brace-delimited languages, for the complexity cap.
//!
//! A function starts at its declaration header and ends at the `}` that
//! closes its body. The header is tracked across lines because formatters
//! wrap long signatures: the line naming the function ends at `(`, and the
//! line carrying the body's `{` may hold only `) -> Type {`. Scanning each
//! line alone named such a function after whatever word preceded a `(` on
//! the brace line (a return type), or — when the brace line had no `(` —
//! missed the header entirely and started a "function" mid-body at the next
//! control-flow line, scoring the wrong span under the wrong name.
//!
//! The lines arrive with comments and literal contents already removed
//! (`source_text`), so braces and branch words inside strings are not
//! counted. Header forms are recognised by `brace_headers`:
//! - a keyword (`fn`) header stays pending until the body's `{` (outside
//!   parentheses and brackets, so `where` clauses and `{` on its own line
//!   work) or a `;` that ends a bodiless declaration.
//! - a name-before-`(` header stays pending while its parameter list is
//!   open. It is dropped when a `{` opens inside that list (a call taking a
//!   callback, whose own named functions must still be found), and when the
//!   list closes the body's `{` must follow on that line or open the next
//!   one (brace-on-next-line style); anything else was a call.
//!
//! A function still open at the end of the file is reported with the score
//! it reached: an unbalanced scan fails closed, not silently.

use super::brace_headers::{keyword_name, name_before_paren};
use super::{FunctionScore, brace_delta, branch_score, normalized_header};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderKind {
    Keyword,
    NameBeforeParen,
}

#[derive(Debug)]
struct Header {
    name: String,
    line: usize,
    kind: HeaderKind,
    /// Open `(` / `[` count at the end of the header's last scanned line.
    depth: i32,
    /// Branch score of the header lines before the body's `{` line.
    score: u32,
    /// The header lines scanned so far.
    text: String,
    /// A name-before-`(` header whose list closed without `{`: only a line
    /// opening with `{` continues it.
    awaiting_brace: bool,
}

#[derive(Debug)]
struct Active {
    function: FunctionScore,
    depth: i32,
}

#[derive(Debug, PartialEq, Eq)]
enum LineOutcome {
    /// The body's `{` is at this byte offset.
    Opens(usize),
    /// A `;` ended a bodiless declaration.
    Declaration,
    /// Neither: the open `(` / `[` count after the line, and whether a `{`
    /// opened inside them.
    Continues { depth: i32, nested_brace: bool },
}

/// Every function in `lines` (comments and literal contents removed), scored
/// from its header line to its closing brace. `rust` disables the
/// name-before-`(` form.
pub(super) fn brace_language_scores<'a>(
    lines: impl Iterator<Item = &'a str>,
    rust: bool,
) -> Vec<FunctionScore> {
    let mut out = Vec::new();
    let mut pending: Option<Header> = None;
    let mut active: Option<Active> = None;
    for (index, line) in lines.enumerate() {
        if active.is_none() {
            active = advance_header(&mut pending, line, index + 1, rust);
        }
        let Some(current) = active.as_mut() else {
            continue;
        };
        current.function.score += branch_score(line);
        current.depth += brace_delta(line);
        if current.depth <= 0
            && let Some(done) = active.take()
        {
            out.push(done.function);
        }
    }
    out.extend(active.map(|open| open.function));
    out
}

/// Feed one line to the header tracker; returns the function whose body
/// opens on this line, if any.
fn advance_header(
    pending: &mut Option<Header>,
    line: &str,
    number: usize,
    rust: bool,
) -> Option<Active> {
    if pending.as_ref().is_some_and(|header| header.awaiting_brace) {
        if let Some(at) = line.find('{')
            && line[..at].trim().is_empty()
        {
            return pending.take().map(|header| open(header, line, at));
        }
        *pending = None;
    }
    let fresh = match pending {
        Some(header) if header.kind == HeaderKind::NameBeforeParen => None,
        Some(_) => keyword_header(line, number),
        None => keyword_header(line, number).or_else(|| paren_header(line, number, rust)),
    };
    let first_line = fresh.is_some();
    if fresh.is_some() {
        *pending = fresh;
    }
    let header = pending.as_mut()?;
    let mut outcome = scan_header_line(line, header.depth);
    // The name-before-`(` form keeps its historical single-line rule: any
    // `{` on the header line opens the body (`describe('x', () => {`).
    if first_line
        && header.kind == HeaderKind::NameBeforeParen
        && let Some(at) = line.find('{')
    {
        outcome = LineOutcome::Opens(at);
    }
    let keep = match outcome {
        LineOutcome::Opens(at) => return pending.take().map(|header| open(header, line, at)),
        LineOutcome::Declaration => false,
        LineOutcome::Continues {
            depth,
            nested_brace,
        } => {
            header.depth = depth;
            match header.kind {
                HeaderKind::Keyword => true,
                HeaderKind::NameBeforeParen if nested_brace => false,
                HeaderKind::NameBeforeParen => {
                    header.awaiting_brace = depth <= 0;
                    true
                }
            }
        }
    };
    if keep {
        header.score += branch_score(line);
        header.text.push_str(line);
    } else {
        *pending = None;
    }
    None
}

fn open(header: Header, line: &str, at: usize) -> Active {
    let text = format!("{}{}", header.text, &line[..at]);
    Active {
        function: FunctionScore {
            name: header.name,
            line: header.line,
            score: 1 + header.score,
            header: normalized_header(&text),
        },
        depth: 0,
    }
}

/// Walk one header line from `depth` open `(` / `[`: a `{` or `;` outside
/// them ends the header.
fn scan_header_line(line: &str, mut depth: i32) -> LineOutcome {
    let mut nested_brace = false;
    for (at, ch) in line.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '{' if depth <= 0 => return LineOutcome::Opens(at),
            '{' => nested_brace = true,
            ';' if depth <= 0 => return LineOutcome::Declaration,
            _ => {}
        }
    }
    LineOutcome::Continues {
        depth,
        nested_brace,
    }
}

fn new_header(name: &str, line: usize, kind: HeaderKind) -> Header {
    Header {
        name: name.to_string(),
        line,
        kind,
        depth: 0,
        score: 0,
        text: String::new(),
        awaiting_brace: false,
    }
}

fn keyword_header(line: &str, number: usize) -> Option<Header> {
    keyword_name(line).map(|name| new_header(name, number, HeaderKind::Keyword))
}

fn paren_header(line: &str, number: usize, rust: bool) -> Option<Header> {
    if rust {
        return None;
    }
    name_before_paren(line).map(|name| new_header(name, number, HeaderKind::NameBeforeParen))
}
