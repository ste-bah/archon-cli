//! Offset-preserving shell regions for the nounset gate.
use regex::Regex;
use std::collections::BTreeSet;
use std::sync::LazyLock;

// Only the body of a quoted here document is literal. Preserve its newlines.
fn heredocs(command: &str) -> Option<(String, Vec<std::ops::Range<usize>>)> {
    static HERE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"<<(-?)[ \t]*(['"]?)([A-Za-z_][A-Za-z0-9_]*)['"]?"#).unwrap()
    });
    let mut bytes = command.as_bytes().to_vec();
    let mut offset = 0;
    let mut literal_code = Vec::new();
    let mut pending: Vec<(String, bool, bool)> = Vec::new();
    for line in command.split_inclusive('\n') {
        if let Some((tag, quoted, tabs)) = pending.first() {
            literal_code.push(offset..offset + line.len());
            if line
                .trim_end_matches('\n')
                .trim_start_matches(if *tabs { "\t" } else { "" })
                == tag
            {
                for ch in &mut bytes[offset..offset + line.len()] {
                    if *ch != b'\n' {
                        *ch = b' ';
                    }
                }
                pending.remove(0);
            } else if *quoted {
                for ch in &mut bytes[offset..offset + line.len()] {
                    if *ch != b'\n' {
                        *ch = b' ';
                    }
                }
            } else {
                // Here document text expands like double quoted text: apostrophes
                // and comment markers are literal, rather than shell syntax.
                for ch in &mut bytes[offset..offset + line.len()] {
                    if matches!(*ch, b'\'' | b'"' | b'#') {
                        *ch = b' ';
                    }
                }
            }
        } else {
            for c in HERE.captures_iter(line) {
                if unquoted_operator(line, c.get(0)?.start()) {
                    pending.push((c[3].into(), !c[2].is_empty(), !c[1].is_empty()));
                }
            }
        }
        offset += line.len();
    }
    if !pending.is_empty() {
        return None;
    }
    Some((String::from_utf8(bytes).ok()?, literal_code))
}

// Ignore redirection-shaped text in quoted arguments and comments.
fn unquoted_operator(line: &str, at: usize) -> bool {
    let bytes = line.as_bytes();
    let (mut i, mut quote) = (0, 0u8);
    while i < at {
        let ch = bytes[i];
        if ch == b'\\' && quote != b'\'' {
            i += 2;
            continue;
        }
        if quote != 0 {
            if ch == quote {
                quote = 0;
            }
        } else if ch == b'\'' || ch == b'"' {
            quote = ch;
        } else if ch == b'#' && (i == 0 || b" \t;|&()".contains(&bytes[i - 1])) {
            return false;
        }
        i += 1;
    }
    quote == 0
}

type Scan = (String, Vec<(String, usize)>, BTreeSet<String>);

// Keep byte offsets and newlines while masking quoted literal text. Only the
// expansion collector sees double quotes, so quoted text cannot invent a set
// command or a binding. Unrecognized parameter syntax fails closed.
pub(super) fn scan(command: &str) -> Option<Scan> {
    let (prepared, literal_code) = heredocs(command)?;
    let b = prepared.as_bytes();
    let mut code: Vec<u8> = b
        .iter()
        .map(|&ch| if ch.is_ascii() { ch } else { b' ' })
        .collect();
    let mut reads = Vec::new();
    let mut defaults = BTreeSet::new();
    let (mut i, mut single, mut double) = (0, false, false);
    while i < b.len() {
        let ch = b[i];
        if ch == b'\n' {
            i += 1;
            continue;
        }
        if single {
            code[i] = b' ';
            if ch == b'\'' {
                single = false;
            }
            i += 1;
            continue;
        }
        if ch == b'\\' {
            code[i] = b' ';
            i += 1;
            if i < b.len() {
                code[i] = b' ';
                i += 1;
            }
            continue;
        }
        if ch == b'\'' && !double {
            single = true;
            code[i] = b' ';
            i += 1;
            continue;
        }
        if ch == b'"' {
            double = !double;
            code[i] = b' ';
            i += 1;
            continue;
        }
        if ch == b'#' && !double && (i == 0 || b" \t\n;|&()".contains(&b[i - 1])) {
            while i < b.len() && b[i] != b'\n' {
                code[i] = b' ';
                i += 1;
            }
            continue;
        }
        if ch == b'`' {
            let start = i;
            i += 1;
            while i < b.len() && b[i] != b'`' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            if i == b.len() {
                return None;
            }
            i += 1;
            for ch in &mut code[start..i] {
                if *ch != b'\n' {
                    *ch = b' ';
                }
            }
            continue;
        }
        if ch == b'$' {
            let start = i;
            i += 1;
            if b.get(i) == Some(&b'(') {
                let body = i;
                let (mut depth, mut quote) = (0usize, 0u8);
                while i < b.len() {
                    let ch = b[i];
                    if ch == b'\\' && quote != b'\'' {
                        i = (i + 2).min(b.len());
                        continue;
                    }
                    if quote != 0 {
                        if ch == quote {
                            quote = 0;
                        }
                    } else {
                        match ch {
                            b'\'' | b'"' => quote = ch,
                            b'(' => depth += 1,
                            b')' => {
                                depth -= 1;
                                if depth == 0 {
                                    i += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    i += 1;
                }
                if depth != 0 {
                    return None;
                }
                // Retain arithmetic binding syntax for the binding collector;
                // substitution output and its quoted literals stay opaque.
                if command[body..i].starts_with("((") {
                    static BIND: LazyLock<Regex> = LazyLock::new(|| {
                        Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*(?:[-+*/%&|^]?=[^=]|\+\+|--)")
                            .unwrap()
                    });
                    for c in BIND.captures_iter(&command[body..i]) {
                        defaults.insert(c[1].into());
                    }
                }
            } else {
                let braced = b.get(i) == Some(&b'{');
                if braced {
                    i += 1;
                }
                let begin = i;
                if b.get(i)
                    .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
                {
                    i += 1;
                    while b
                        .get(i)
                        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
                    {
                        i += 1;
                    }
                    let name = command[begin..i].to_string();
                    if braced && b.get(i) != Some(&b'}') {
                        if b.get(i) == Some(&b':') {
                            i += 1;
                        }
                        if !b.get(i).is_some_and(|c| b"-=+?".contains(c)) {
                            return None;
                        }
                        if b.get(i) == Some(&b'?') {
                            reads.push((name.clone(), start));
                        }
                        defaults.insert(name);
                        while i < b.len() && b[i] != b'}' {
                            if b"${\n".contains(&b[i]) {
                                return None;
                            }
                            i += 1;
                        }
                        if b.get(i) != Some(&b'}') {
                            return None;
                        }
                    } else {
                        reads.push((name, start));
                    }
                    if braced {
                        i += 1;
                    }
                } else if braced {
                    return None;
                }
            }
            for c in &mut code[start..i] {
                if *c != b'\n' {
                    *c = b' ';
                }
            }
            continue;
        }
        if double {
            code[i] = b' ';
        }
        i += 1;
    }
    if single || double {
        return None;
    }
    for range in literal_code {
        for ch in &mut code[range] {
            if *ch != b'\n' {
                *ch = b' ';
            }
        }
    }
    Some((String::from_utf8(code).ok()?, reads, defaults))
}
