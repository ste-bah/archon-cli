//! Issue 361 (tests only): the tokens of one Rust source file, so a comment
//! or a string literal never fakes or hides a reference, the token-level
//! readers of its attributes, `use` trees and `impl` headers, and the text
//! the logic digest is taken over.
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Tok {
    Ident(String),
    Str(String),
    PathSep,
    Punct(char),
}

/// The tokens of `text`, and the offsets where a `//` comment starts.
pub(crate) fn lex(text: &str) -> (Vec<Tok>, BTreeSet<usize>) {
    let b = text.as_bytes();
    let (mut i, mut toks, mut comments) = (0, Vec::new(), BTreeSet::new());
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    while i < b.len() {
        let c = b[i];
        let next = b.get(i + 1).copied();
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'/' && next == Some(b'/') {
            comments.insert(i);
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && next == Some(b'*') {
            let mut depth = 0usize;
            while i < b.len() {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    (depth, i) = (depth + 1, i + 2);
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    (depth, i) = (depth - 1, i + 2);
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if c == b'"' {
            let start = i + 1;
            i = start;
            while i < b.len() && b[i] != b'"' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            toks.push(Tok::Str(text[start..i.min(b.len())].to_string()));
            i += 1;
        } else if c == b'\'' {
            if next == Some(b'\\') {
                i += 3;
                while i < b.len() && b[i] != b'\'' {
                    i += 1;
                }
                i += 1;
            } else {
                let width = text[i + 1..].chars().next().map_or(1, char::len_utf8);
                if b.get(i + 1 + width) == Some(&b'\'') {
                    i += 2 + width;
                } else {
                    // A lifetime or a label: never a name.
                    i += 1;
                    while i < b.len() && ident(b[i]) {
                        i += 1;
                    }
                }
            }
        } else if c.is_ascii_digit() {
            while i < b.len() && ident(b[i]) {
                i += 1;
            }
        } else if ident(c) {
            let start = i;
            while i < b.len() && ident(b[i]) {
                i += 1;
            }
            let word = &text[start..i];
            let hashes = b[i..].iter().take_while(|&&h| h == b'#').count();
            if matches!(word, "r" | "br" | "cr") && b.get(i + hashes) == Some(&b'"') {
                let close = format!("\"{}", "#".repeat(hashes));
                let body = i + hashes + 1;
                let end = text[body..].find(&close).map_or(b.len(), |at| body + at);
                toks.push(Tok::Str(text[body..end].to_string()));
                i = (end + close.len()).min(b.len());
            } else if word == "r" && hashes == 1 {
                let start = i + 1;
                i = start;
                while i < b.len() && ident(b[i]) {
                    i += 1;
                }
                toks.push(Tok::Ident(text[start..i].to_string()));
            } else {
                toks.push(Tok::Ident(word.to_string()));
            }
        } else if c == b':' && next == Some(b':') {
            toks.push(Tok::PathSep);
            i += 2;
        } else {
            let ch = text[i..].chars().next().unwrap_or('?');
            toks.push(Tok::Punct(ch));
            i += ch.len_utf8();
        }
    }
    (toks, comments)
}

/// The text the digest reads: LF line ends, without whole-line `//`
/// comments and without a trailing `#[cfg(test)] mod … { … }` block, so a
/// comment or an in-file test edit is not a logic change. A line inside a
/// string literal is kept whatever it starts with.
pub(crate) fn hashed_text(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let (_, comments) = lex(&text);
    let mut kept = Vec::new();
    let mut offset = 0;
    for line in text.split('\n') {
        let start = offset + (line.len() - line.trim_start().len());
        offset += line.len() + 1;
        if !comments.contains(&start) {
            // The pin is the output of this digest. Hashing its current
            // value would make every legitimate re-pin change the digest
            // again, so keep the field and its shape while normalizing only
            // the pinned bytes.
            if line.trim_start().starts_with("sources_digest: ") {
                if let (Some(open), Some(close)) = (line.find('"'), line.rfind('"')) {
                    if close > open {
                        let mut normalized = line[..=open].to_owned();
                        normalized.push_str("<source-digest>");
                        normalized.push_str(&line[close..]);
                        kept.push(normalized);
                        continue;
                    }
                }
            }
            kept.push(line.to_owned());
        }
    }
    let opener = |line: &str| {
        let line = ["pub(crate) ", "pub(super) ", "pub "]
            .iter()
            .find_map(|prefix| line.strip_prefix(prefix))
            .unwrap_or(line);
        line.starts_with("mod ") && line.ends_with(" {")
    };
    let last = kept.iter().rposition(|line| !line.trim().is_empty());
    let cut = kept.iter().enumerate().find_map(|(at, line)| {
        let block = *line == "#[cfg(test)]" && kept.get(at + 1).is_some_and(|l| opener(l));
        // Only a line that is the brace alone closes it: code after the
        // brace on that line is hashed.
        let close = kept[at + 1..].iter().position(|l| l.trim_end() == "}");
        (block && close.map(|close| at + 1 + close) == last).then_some(at)
    });
    kept.truncate(cut.unwrap_or(kept.len()));
    kept.join("\n")
}

pub(crate) const KEYWORDS: &[&str] = &[
    "as",
    "async",
    "await",
    "break",
    "const",
    "continue",
    "crate",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "fn",
    "for",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
    "union",
    "macro_rules",
    "_",
    "default",
];

pub(crate) fn word(tok: Option<&Tok>) -> Option<&str> {
    match tok {
        Some(Tok::Ident(word)) => Some(word),
        _ => None,
    }
}

pub(crate) fn is(tok: Option<&Tok>, c: char) -> bool {
    tok == Some(&Tok::Punct(c))
}

/// The index after the item starting at `i`.
pub(crate) fn skip_item(toks: &[Tok], mut i: usize) -> usize {
    let mut depth = 0i32;
    while i < toks.len() {
        match &toks[i] {
            Tok::Punct('{' | '(' | '[') => depth += 1,
            Tok::Punct('}' | ')' | ']') => {
                depth -= 1;
                if depth == 0 && toks[i] == Tok::Punct('}') {
                    return i + 1;
                }
            }
            Tok::Punct(';') if depth == 0 => return i + 1,
            _ => {}
        }
        i += 1;
    }
    i
}

/// The index after the statement, field or expression starting at `i`;
/// a closing bracket that is not its own stays unread.
pub(crate) fn skip_statement(toks: &[Tok], mut i: usize) -> usize {
    let mut depth = 0i32;
    while i < toks.len() {
        match &toks[i] {
            Tok::Punct('{' | '(' | '[') => depth += 1,
            Tok::Punct('}' | ')' | ']') => {
                depth -= 1;
                if depth < 0 {
                    return i;
                }
                if depth == 0 && toks[i] == Tok::Punct('}') {
                    return i + 1;
                }
            }
            Tok::Punct(';' | ',') if depth == 0 => return i + 1,
            _ => {}
        }
        i += 1;
    }
    i
}

/// The tokens of the attribute opening at `i` (`#` `[` …), and the index
/// after it.
pub(crate) fn attribute(toks: &[Tok], i: usize) -> (&[Tok], usize) {
    let open = if is(toks.get(i + 1), '!') {
        i + 2
    } else {
        i + 1
    };
    let mut depth = 0;
    for at in open..toks.len() {
        match toks[at] {
            Tok::Punct('[') => depth += 1,
            Tok::Punct(']') => {
                depth -= 1;
                if depth == 0 {
                    return (&toks[open + 1..at], at + 1);
                }
            }
            _ => {}
        }
    }
    (&toks[open..], toks.len())
}

pub(crate) fn test_cfg(attr: &[Tok]) -> bool {
    let words = attr
        .iter()
        .take(5)
        .map(|t| word(Some(t)))
        .collect::<Vec<_>>();
    words.first() == Some(&Some("cfg"))
        && (attr.len() == 4 && words.get(2) == Some(&Some("test"))
            || words.get(2) == Some(&Some("all")) && words.get(4) == Some(&Some("test")))
}

pub(crate) type Leaves = Vec<(Vec<String>, Option<String>)>;

pub(crate) fn use_tree(
    toks: &[Tok],
    mut i: usize,
    mut path: Vec<String>,
    out: &mut Leaves,
) -> usize {
    if toks.get(i) == Some(&Tok::PathSep) {
        path.push(String::new());
        i += 1;
    }
    loop {
        match toks.get(i) {
            Some(Tok::Ident(seg)) => {
                path.push(seg.clone());
                i += 1;
                if toks.get(i) == Some(&Tok::PathSep) {
                    i += 1;
                    continue;
                }
                if path.last().map(String::as_str) == Some("self") {
                    path.pop();
                }
                let mut alias = path.last().cloned();
                if word(toks.get(i)) == Some("as") {
                    alias = word(toks.get(i + 1)).map(str::to_string);
                    i += 2;
                }
                out.push((path, alias));
                return i;
            }
            Some(Tok::Punct('*')) => {
                out.push((path, None));
                return i + 1;
            }
            Some(Tok::Punct('{')) => {
                i += 1;
                while i < toks.len() && !is(toks.get(i), '}') {
                    i = use_tree(toks, i, path.clone(), out);
                    if is(toks.get(i), ',') {
                        i += 1;
                    }
                }
                return i + 1;
            }
            _ => return i + 1,
        }
    }
}

/// The type an `impl` header starting after `impl` at `i` is for.
pub(crate) fn impl_type(toks: &[Tok], i: usize) -> Option<String> {
    let (mut angle, mut last) = (0i32, None::<&str>);
    for at in i..toks.len() {
        match &toks[at] {
            Tok::Punct('{') | Tok::Punct(';') => break,
            Tok::Ident(w) if w == "where" => break,
            Tok::Punct('<') => angle += 1,
            Tok::Punct('>') if !is(toks.get(at.wrapping_sub(1)), '-') => angle -= 1,
            Tok::Ident(w) if w == "for" && angle == 0 => last = None,
            Tok::Ident(w) if angle == 0 && !KEYWORDS.contains(&w.as_str()) => last = Some(w),
            _ => {}
        }
    }
    last.map(str::to_string)
}

pub(crate) fn include_target(toks: &[Tok], i: usize) -> Option<(bool, String)> {
    match (toks.get(i), toks.get(i + 1)) {
        (Some(Tok::Str(path)), Some(Tok::Punct(')'))) => Some((false, path.clone())),
        (Some(Tok::Ident(c)), _) if c == "concat" => {
            let manifest = toks.get(i + 5) == Some(&Tok::Str("CARGO_MANIFEST_DIR".into()));
            match (manifest, toks.get(i + 8), toks.get(i + 9)) {
                (true, Some(Tok::Str(path)), Some(Tok::Punct(')'))) => Some((true, path.clone())),
                _ => None,
            }
        }
        _ => None,
    }
}
