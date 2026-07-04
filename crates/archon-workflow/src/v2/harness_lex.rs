fn skip_function_declaration(source: &str, start: usize) -> Option<usize> {
    let open = source[start..].find('{')? + start;
    matching_delimiter(source, open, '{', '}').map(|idx| idx + 1)
}

fn matching_delimiter(source: &str, open_idx: usize, open: char, close: char) -> Option<usize> {
    let mut idx = open_idx;
    let mut depth = 0usize;
    while idx < source.len() {
        let ch = source[idx..].chars().next()?;
        if matches!(ch, '"' | '\'' | '`') {
            idx = skip_quoted(source, idx, ch);
            continue;
        }
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(idx);
            }
        }
        idx += ch.len_utf8();
    }
    None
}

fn skip_quoted(source: &str, start: usize, quote: char) -> usize {
    let mut idx = start + quote.len_utf8();
    let mut escaped = false;
    while idx < source.len() {
        let ch = source[idx..]
            .chars()
            .next()
            .expect("quote index on char boundary");
        idx += ch.len_utf8();
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            break;
        }
    }
    idx
}

fn skip_ws(source: &str, mut idx: usize) -> usize {
    while idx < source.len() {
        let ch = source[idx..]
            .chars()
            .next()
            .expect("whitespace index on char boundary");
        if !ch.is_whitespace() {
            break;
        }
        idx += ch.len_utf8();
    }
    idx
}

fn take_ident(source: &str, mut idx: usize) -> usize {
    while idx < source.len() {
        let ch = source[idx..]
            .chars()
            .next()
            .expect("identifier index on char boundary");
        if !(ch == '_' || ch.is_ascii_alphanumeric()) {
            break;
        }
        idx += ch.len_utf8();
    }
    idx
}

fn starts_keyword(source: &str, idx: usize, keyword: &str) -> bool {
    source[idx..].starts_with(keyword)
        && source[idx + keyword.len()..]
            .chars()
            .next()
            .is_none_or(|ch| !(ch == '_' || ch.is_ascii_alphanumeric()))
        && is_boundary_before(source, idx)
}

fn is_boundary_before(source: &str, idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    source[..idx]
        .chars()
        .next_back()
        .is_none_or(|ch| !(ch == '_' || ch.is_ascii_alphanumeric()))
}

fn binding_before_call(source: &str, call_start: usize) -> Option<String> {
    let prefix = &source[..call_start];
    let statement_start = prefix
        .rfind(';')
        .or_else(|| prefix.rfind('\n'))
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let statement = prefix[statement_start..].trim();
    let declaration =
        Regex::new(r#"(?:const|let|var)\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:await\s*)?$"#)
            .expect("declaration binding regex compiles");
    if let Some(binding) = declaration
        .captures(statement)
        .and_then(|captures| captures.get(1))
        .map(|m| m.as_str().to_string())
    {
        return Some(binding);
    }
    let reassignment = Regex::new(r#"^([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:await\s*)?$"#)
        .expect("binding regex compiles");
    reassignment
        .captures(statement)
        .and_then(|captures| captures.get(1))
        .map(|m| m.as_str().to_string())
}
