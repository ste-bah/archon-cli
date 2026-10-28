use super::Tok;
use std::collections::BTreeSet;

pub(super) fn lex_names(text: &str) -> (Vec<Tok>, BTreeSet<usize>, Vec<usize>) {
    let b = text.as_bytes();
    let mut offsets = Vec::new();
    let (mut i, mut toks, mut comments) = (0, Vec::new(), BTreeSet::new());
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    while i < b.len() {
        let start = i;
        let count = toks.len();
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
            toks.push(Tok::Str(decode(&text[start..i.min(b.len())])));
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
            let start = i;
            while i < b.len() && ident(b[i]) {
                i += 1;
            }
            toks.push(Tok::Number(text[start..i].to_string()));
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
        if toks.len() != count {
            offsets.push(start);
        }
    }
    let newlines = text
        .match_indices('\n')
        .map(|(at, _)| at)
        .collect::<Vec<_>>();
    let lines = offsets
        .into_iter()
        .map(|at| newlines.partition_point(|&nl| nl < at) + 1)
        .collect();
    (toks, comments, lines)
}

fn decode(text: &str) -> String {
    let mut chars = text.chars().peekable();
    let mut out = String::new();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next().expect("incomplete escape") {
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '0' => out.push('\0'),
            'x' => {
                let hex = chars.by_ref().take(2).collect::<String>();
                out.push(char::from(
                    u8::from_str_radix(&hex, 16).expect("invalid escape"),
                ));
            }
            'u' => {
                assert_eq!(chars.next(), Some('{'));
                let hex = chars
                    .by_ref()
                    .take_while(|ch| *ch != '}')
                    .filter(|ch| *ch != '_')
                    .collect::<String>();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
            }
            '\n' | '\r' => {
                while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
                    chars.next();
                }
            }
            other => panic!("unsupported escape: {other}"),
        }
    }
    out
}
