//! Regions of a parsed file the hand scanner reads instead of the tree.
//!
//! - A Rust macro invocation or `macro_rules!` body is a flat token tree to
//!   the parser.
//! - A C / C++ `function_definition` the parser made out of a class — an
//!   export or alignment macro in the class head (`class API_EXPORT Foo {`,
//!   `class ALIGNAS(16) Foo {`) — or one with no declarator name at all.
//!   A misread class is read method by method: the class body is found by
//!   matching braces over the file text (the tree's node may end mid-class)
//!   and only the body is scanned, so the class head is never read as a
//!   function header.
//!
//! A region's byte range is recorded so the tree walk skips whatever the
//! parser made of the same text, and nothing is judged twice. A reading that
//! lost sync is not judged at all and is returned as a note.

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;
use tree_sitter::Node;

use super::brace_scan::brace_language_scores;
use super::source_text::{CodeLine, Syntax, code_lines};
use super::tree_scan::TreeScan;
use super::{FunctionScore, hand_scan};

/// A class head the parser misreads: `class|struct|union`, a macro word
/// with optional arguments, the class name, optional `final` and bases.
static MISREAD_CLASS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(template\s*<.*>\s*)?(class|struct|union)\s+[A-Za-z_]\w*(\s*\([^()]*\))?\s+[A-Za-z_]\w*(\s+final)?\s*(:.*)?$",
    )
    .expect("class head pattern")
});

const C_FAMILY: Syntax = Syntax::CFamily {
    preprocessor: true,
    raw_backticks: false,
};

/// Whether a C / C++ `function_definition` is a class the parser misread.
pub(super) fn misread_class(node: Node, text: &str) -> bool {
    let body = node
        .child_by_field_name("body")
        .map_or(node.end_byte(), |body| body.start_byte());
    let head: String = code_lines(&text[node.start_byte()..body], C_FAMILY)
        .iter()
        .map(|line| line.code.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let head: Vec<&str> = head.split_whitespace().collect();
    MISREAD_CLASS.is_match(&head.join(" "))
}

/// Read a misread or unnamed C / C++ definition starting at `node`. The
/// text is lexed once from the definition's start, so a comment or raw
/// string opened on the class-head line is still one when the body is read,
/// and a misread class is read from the byte after its `{` to the byte
/// before its `}`.
pub(super) fn scan_c_definition(
    node: Node,
    text: &str,
    class: bool,
    out: &mut TreeScan,
    scanned: &mut Vec<Range<usize>>,
) {
    let start = node.start_byte();
    let head = &text[start..];
    let row = node.start_position().row;
    let reason = if class {
        "the parser read a class as one function (a macro in its head); its methods were read by \
         the hand scanner instead"
    } else {
        "the parser read this as a function without a name (often a macro); read by the hand \
         scanner instead"
    };
    out.notes.push((row + 1, reason.to_string()));
    let lines = code_lines(head, C_FAMILY);
    let Some(((open, open_at), (close, close_at))) = outer_block(&lines) else {
        scanned.push(node.byte_range());
        scan_text(&text[node.byte_range()], "region.cpp", row, out);
        return;
    };
    let body: Vec<CodeLine> = if !class {
        lines[..=close].to_vec()
    } else if open == close {
        vec![lines[open].slice(open_at + 1, close_at)]
    } else {
        let mut body = vec![lines[open].slice(open_at + 1, usize::MAX)];
        body.extend(lines[open + 1..close].iter().cloned());
        body.push(lines[close].slice(0, close_at));
        body
    };
    let offset = if class { row + open } else { row };
    read_lines(&body, offset, out);
    let end = start
        + head
            .split_inclusive('\n')
            .take(close + 1)
            .map(str::len)
            .sum::<usize>();
    scanned.push(start..end.max(node.end_byte()));
}

/// Judge the functions the brace scanner finds in already-lexed `lines`, the
/// first of which is 0-based row `offset + 0` of the file; a reading that
/// lost sync is noted, not judged.
fn read_lines(lines: &[CodeLine], offset: usize, out: &mut TreeScan) {
    let scan = brace_language_scores(lines, false);
    if let Some((name, line)) = scan.unclosed {
        let reason = format!(
            "in a region the hand scanner read: scanner lost sync: function '{name}' never closed"
        );
        out.notes.push((line + offset, reason));
        return;
    }
    out.functions
        .extend(scan.functions.into_iter().map(|mut function| {
            function.line += offset;
            function.end_line += offset;
            function
        }));
}

/// Read a Rust macro's text.
pub(super) fn scan_macro(node: Node, text: &str, out: &mut TreeScan) {
    scan_text(
        &text[node.byte_range()],
        "macro.rs",
        node.start_position().row,
        out,
    );
}

/// (line, code byte offset) of the first `{` in `lines` and of its matching
/// `}`.
fn outer_block(lines: &[CodeLine]) -> Option<((usize, usize), (usize, usize))> {
    let mut depth = 0i32;
    let mut open = None;
    for (index, line) in lines.iter().enumerate() {
        for (at, ch) in line.code.char_indices() {
            match ch {
                '{' => {
                    open.get_or_insert((index, at));
                    depth += 1;
                }
                '}' if open.is_some() => {
                    depth -= 1;
                    if depth == 0 {
                        return open.map(|open| (open, (index, at)));
                    }
                }
                _ => {}
            }
        }
    }
    None
}

fn scan_text(body: &str, as_path: &str, offset: usize, out: &mut TreeScan) {
    let (functions, notes) = region_functions(body, as_path, offset);
    out.functions.extend(functions);
    out.notes.extend(notes);
}

/// A Rust macro body read by the hand scanner (see [`region_functions`]).
#[cfg(test)]
pub(super) fn macro_functions(
    body: &str,
    offset: usize,
) -> (Vec<FunctionScore>, Vec<(usize, String)>) {
    region_functions(body, "macro.rs", offset)
}

/// Functions the hand scanner finds in `body`, which starts on 0-based row
/// `offset`. A reading that lost sync is not judged at all — every span in
/// it is suspect — and is returned as a note instead.
fn region_functions(
    body: &str,
    as_path: &str,
    offset: usize,
) -> (Vec<FunctionScore>, Vec<(usize, String)>) {
    let scan = hand_scan(as_path, body);
    let notes = scan
        .unreliable
        .into_iter()
        .map(|(line, reason)| {
            let reason = format!("in a region the hand scanner read: {reason}");
            (line + offset, reason)
        })
        .collect();
    if scan.lost_sync {
        return (Vec::new(), notes);
    }
    let functions = scan
        .functions
        .into_iter()
        .map(|mut function| {
            function.line += offset;
            function.end_line += offset;
            function
        })
        .collect();
    (functions, notes)
}
