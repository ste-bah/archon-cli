//! Mask Rust literals and comments once, across line boundaries, preserving line numbers.
pub(super) fn code(source: &str) -> String {
    let input = source.as_bytes();
    let mut out = input.to_vec();
    let mut at = 0;
    while at < input.len() {
        let start = at;
        if input[at..].starts_with(b"//") {
            while at < input.len() && input[at] != b'\n' {
                at += 1;
            }
        } else if input[at..].starts_with(b"/*") {
            at += 2;
            let mut depth = 1;
            while at < input.len() && depth > 0 {
                if input[at..].starts_with(b"/*") {
                    depth += 1;
                    at += 2;
                } else if input[at..].starts_with(b"*/") {
                    depth -= 1;
                    at += 2;
                } else {
                    at += 1;
                }
            }
        } else if input[at] == b'r' && raw_quote(input, at).is_some() {
            let quote = raw_quote(input, at).unwrap();
            let hashes = quote - at - 1;
            at = quote + 1;
            while at < input.len() {
                if input[at] == b'"'
                    && input
                        .get(at + 1..at + 1 + hashes)
                        .is_some_and(|end| end.iter().all(|c| *c == b'#'))
                {
                    at += 1 + hashes;
                    break;
                }
                at += 1;
            }
        } else if input[at] == b'"' {
            at += 1;
            while at < input.len() {
                let c = input[at];
                at += 1;
                if c == b'\\' {
                    at = (at + 1).min(input.len());
                } else if c == b'"' {
                    break;
                }
            }
        } else if input[at] == b'\'' && char_end(input, at).is_some() {
            at = char_end(input, at).unwrap();
        } else {
            at += 1;
            continue;
        }
        for byte in &mut out[start..at] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
    }
    String::from_utf8(out).expect("masking entire literals preserves UTF-8")
}

fn raw_quote(input: &[u8], at: usize) -> Option<usize> {
    let mut quote = at + 1;
    while input.get(quote) == Some(&b'#') {
        quote += 1;
    }
    (input.get(quote) == Some(&b'"')).then_some(quote)
}

fn char_end(input: &[u8], at: usize) -> Option<usize> {
    let mut end = at + 1;
    if input.get(end) == Some(&b'\\') {
        end += 2;
        if input.get(end - 1) == Some(&b'u') {
            while input.get(end).is_some_and(|c| *c != b'}' && *c != b'\n') {
                end += 1;
            }
            end += 1;
        } else if input.get(end - 1) == Some(&b'x') {
            end += 2;
        }
    } else {
        end += 1;
        while input.get(end).is_some_and(|c| c & 0xc0 == 0x80) {
            end += 1;
        }
    }
    (input.get(end) == Some(&b'\'')).then_some(end + 1)
}
