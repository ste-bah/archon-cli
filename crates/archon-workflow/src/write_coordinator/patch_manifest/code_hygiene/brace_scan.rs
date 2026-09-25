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
//! Two header forms are recognised:
//! - keyword headers: `fn name(` / `fn name<..>(` behind any Rust qualifiers
//!   (`pub`, `pub(crate)`, `async`, `const`, `unsafe`, `extern "C"`, ...).
//!   They stay pending until the body's `{` (outside parentheses and
//!   brackets) or a `;` that ends a bodiless declaration.
//! - the name-before-`(` form other brace languages use (`int name(`,
//!   `function name(`, `public Foo name(`). It never applies to `.rs` files,
//!   where every function is a keyword header and the form only ever matched
//!   control flow. It stays pending only while its parameter list is still
//!   open, so a multi-line call is dropped when its `)` arrives without a
//!   `{`.

use super::{FunctionScore, brace_delta, branch_score, valid_name};

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
}

#[derive(Debug)]
struct Active {
    name: String,
    line: usize,
    depth: i32,
    score: u32,
}

#[derive(Debug, PartialEq, Eq)]
enum LineOutcome {
    /// The body's `{` is on this line.
    Opens,
    /// A `;` ended a bodiless declaration.
    Declaration,
    /// Neither; the open `(` / `[` count after the line.
    Continues(i32),
}

/// Leading words after which a name-before-`(` line is control flow, not a
/// declaration (`if let Some(x) = f(y) {`, `when (x) {`, `guard let ...`).
const CONTROL_WORDS: &[&str] = &[
    "if", "else", "for", "while", "do", "loop", "match", "switch", "case", "catch", "try",
    "return", "when", "guard", "throw",
];

/// Qualifiers that may precede `fn` in a Rust declaration.
const FN_QUALIFIERS: &[&str] = &["async", "const", "unsafe", "safe", "extern", "default"];

/// Every function in `lines` (comment tails already stripped), scored from
/// its header line to its closing brace. `rust` disables the
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
        current.score += branch_score(line);
        current.depth += brace_delta(line);
        if current.depth <= 0
            && let Some(done) = active.take()
        {
            out.push(FunctionScore {
                name: done.name,
                line: done.line,
                score: done.score,
            });
        }
    }
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
    let fresh = match pending {
        Some(header) if header.kind == HeaderKind::NameBeforeParen => None,
        Some(_) => keyword_header(line, number),
        None => {
            keyword_header(line, number).or_else(|| name_before_paren_header(line, number, rust))
        }
    };
    let first_line = fresh.is_some();
    if fresh.is_some() {
        *pending = fresh;
    }
    let header = pending.as_mut()?;
    // A Rust signature's continuation lines carry no string literal; one that
    // does means the `fn name(` was text inside a string, not a header.
    if !first_line && header.kind == HeaderKind::Keyword && line.contains('"') {
        *pending = None;
        return None;
    }
    let outcome = scan_header_line(line, header.depth);
    // The name-before-`(` form keeps its historical single-line rule: any
    // `{` on the header line opens the body (`describe('x', () => {`).
    let legacy_open =
        first_line && header.kind == HeaderKind::NameBeforeParen && line.contains('{');
    if legacy_open || outcome == LineOutcome::Opens {
        let header = pending.take()?;
        return Some(Active {
            name: header.name,
            line: header.line,
            depth: 0,
            score: 1 + header.score,
        });
    }
    match outcome {
        LineOutcome::Continues(depth) if header.kind == HeaderKind::Keyword || depth > 0 => {
            header.depth = depth;
            header.score += branch_score(line);
        }
        _ => *pending = None,
    }
    None
}

/// Walk one header line from `depth` open `(` / `[`: a `{` or `;` outside
/// them ends the header.
fn scan_header_line(line: &str, mut depth: i32) -> LineOutcome {
    for ch in line.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '{' if depth <= 0 => return LineOutcome::Opens,
            ';' if depth <= 0 => return LineOutcome::Declaration,
            _ => {}
        }
    }
    LineOutcome::Continues(depth)
}

fn new_header(name: &str, line: usize, kind: HeaderKind) -> Header {
    Header {
        name: name.to_string(),
        line,
        kind,
        depth: 0,
        score: 0,
    }
}

/// `fn name(` or `fn name<..>(`, with only Rust qualifiers before `fn`.
fn keyword_header(line: &str, number: usize) -> Option<Header> {
    let trimmed = line.trim_start();
    let (prefix, rest) = match trimmed.strip_prefix("fn ") {
        Some(rest) => ("", rest),
        None => trimmed.split_once(" fn ")?,
    };
    if !only_fn_qualifiers(prefix) {
        return None;
    }
    let rest = rest.trim_start();
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    let (name, tail) = rest.split_at(end);
    let tail = tail.trim_start();
    let opens_params = tail.starts_with('(') || tail.starts_with('<');
    (!name.is_empty() && opens_params).then(|| new_header(name, number, HeaderKind::Keyword))
}

fn only_fn_qualifiers(prefix: &str) -> bool {
    let mut rest = prefix.trim();
    if let Some(after) = rest.strip_prefix("pub") {
        rest = match after.trim_start().strip_prefix('(') {
            Some(scope) => match scope.split_once(')') {
                Some((_, tail)) => tail,
                None => return false,
            },
            None => after,
        };
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return false;
        }
    }
    rest.split_whitespace().all(|word| {
        FN_QUALIFIERS.contains(&word)
            || (word.len() >= 2 && word.starts_with('"') && word.ends_with('"'))
    })
}

/// The last word before the first `(` (`int name(`, `function name(`),
/// unless the line is control flow, continues a parameter list, or the word
/// is cut inside a generic argument list (`Result<(), E>` gives `Result<`).
fn name_before_paren_header(line: &str, number: usize, rust: bool) -> Option<Header> {
    if rust {
        return None;
    }
    let trimmed = line.trim_start();
    if trimmed.starts_with(')') {
        return None;
    }
    let before = trimmed.split_once('(')?.0.trim();
    let first = before.trim_start_matches('}').split_whitespace().next()?;
    if CONTROL_WORDS.contains(&first) {
        return None;
    }
    let name = before.split_whitespace().last()?;
    let balanced = name.matches('<').count() == name.matches('>').count();
    (balanced && valid_name(name)).then(|| new_header(name, number, HeaderKind::NameBeforeParen))
}
