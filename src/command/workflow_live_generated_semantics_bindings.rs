use archon_workflow::{WorkflowError, WorkflowResult};

pub(super) fn validate_const_host_call_bindings(source: &str) -> WorkflowResult<()> {
    let code = mask_strings_and_comments(source);
    for binding in const_host_call_bindings(&code) {
        if has_rebinding(&code, &binding.name, binding.scan_start) {
            return Err(WorkflowError::SpecInvalid(format!(
                "generated workflow.js must not reassign const host-call results; `{}` must use `let` before normalization or repair",
                binding.name
            )));
        }
    }
    Ok(())
}

struct ConstHostCallBinding {
    name: String,
    scan_start: usize,
}

fn const_host_call_bindings(code: &str) -> Vec<ConstHostCallBinding> {
    let mut bindings = Vec::new();
    let mut offset = 0usize;
    while let Some(index) = find_ident(code, "const", offset) {
        let Some((name, name_end, value_start)) = parse_const_assignment(code, index + 5) else {
            offset = index + 5;
            continue;
        };
        if starts_with_await_host_call(code, value_start) {
            bindings.push(ConstHostCallBinding {
                name: name.to_string(),
                scan_start: statement_end(code, value_start),
            });
        }
        offset = name_end;
    }
    bindings
}

fn parse_const_assignment(code: &str, start: usize) -> Option<(&str, usize, usize)> {
    let name_start = skip_ws(code, start);
    let name_end = ident_end(code, name_start)?;
    let equals = skip_ws(code, name_end);
    if code.as_bytes().get(equals).copied() != Some(b'=') {
        return None;
    }
    Some((
        code.get(name_start..name_end)?,
        name_end,
        skip_ws(code, equals + 1),
    ))
}

fn starts_with_await_host_call(code: &str, start: usize) -> bool {
    let Some(after_await) = consume_word(code, start, "await") else {
        return false;
    };
    let after_await = skip_ws(code, after_await);
    let Some(after_w) = consume_word(code, after_await, "w") else {
        return false;
    };
    let method_start = skip_ws(code, after_w);
    ["reduce", "parallel", "fanout"].iter().any(|method| {
        code.as_bytes().get(method_start) == Some(&b'.')
            && consume_word(code, method_start + 1, method).is_some()
    })
}

fn has_rebinding(code: &str, name: &str, start: usize) -> bool {
    let mut offset = start;
    while let Some(index) = find_ident(code, name, offset) {
        if !declaration_precedes(code, index) && assignment_follows(code, index + name.len()) {
            return true;
        }
        offset = index + name.len();
    }
    false
}

fn declaration_precedes(code: &str, index: usize) -> bool {
    let before = code.get(..index).unwrap_or_default().trim_end();
    ["const", "let", "var"]
        .iter()
        .any(|keyword| before.ends_with(keyword) && declaration_boundary(before, keyword.len()))
}

fn declaration_boundary(before: &str, keyword_len: usize) -> bool {
    before
        .len()
        .checked_sub(keyword_len + 1)
        .and_then(|index| before.as_bytes().get(index))
        .is_none_or(|byte| !is_ident_continue(*byte))
}

fn assignment_follows(code: &str, start: usize) -> bool {
    let index = skip_ws(code, start);
    let tail = code.get(index..).unwrap_or_default();
    if tail.starts_with('.') || tail.starts_with("?.") {
        return false;
    }
    tail.starts_with('=') && !tail.starts_with("==") && !tail.starts_with("=>")
        || ["+=", "-=", "*=", "/=", "%=", "&&=", "||=", "??="]
            .iter()
            .any(|op| tail.starts_with(op))
}

fn mask_strings_and_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut index = 0usize;
    while index < bytes.len() {
        index = match bytes[index] {
            b'\'' | b'"' | b'`' => mask_quoted(bytes, &mut out, index, bytes[index]),
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                mask_line_comment(bytes, &mut out, index)
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                mask_block_comment(bytes, &mut out, index)
            }
            _ => index + 1,
        };
    }
    String::from_utf8(out).unwrap_or_default()
}

fn mask_quoted(bytes: &[u8], out: &mut [u8], start: usize, quote: u8) -> usize {
    out[start] = b' ';
    let mut index = start + 1;
    let mut escaped = false;
    while index < bytes.len() {
        out[index] = masked_byte(bytes[index]);
        if escaped {
            escaped = false;
        } else if bytes[index] == b'\\' {
            escaped = true;
        } else if bytes[index] == quote {
            return index + 1;
        }
        index += 1;
    }
    index
}

fn mask_line_comment(bytes: &[u8], out: &mut [u8], start: usize) -> usize {
    let mut index = start;
    while index < bytes.len() && bytes[index] != b'\n' {
        out[index] = b' ';
        index += 1;
    }
    index
}

fn mask_block_comment(bytes: &[u8], out: &mut [u8], start: usize) -> usize {
    let mut index = start;
    while index < bytes.len() {
        out[index] = masked_byte(bytes[index]);
        if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
            out[index + 1] = b' ';
            return index + 2;
        }
        index += 1;
    }
    index
}

fn masked_byte(byte: u8) -> u8 {
    if byte == b'\n' { b'\n' } else { b' ' }
}

fn statement_end(code: &str, start: usize) -> usize {
    code.get(start..)
        .and_then(|tail| tail.find(';').map(|relative| start + relative + 1))
        .unwrap_or(start)
}

fn skip_ws(code: &str, start: usize) -> usize {
    code.as_bytes()
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, byte)| !byte.is_ascii_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(code.len())
}

fn consume_word(code: &str, start: usize, word: &str) -> Option<usize> {
    let end = start + word.len();
    if code.get(start..end) == Some(word) && ident_boundary(code, start, end) {
        Some(end)
    } else {
        None
    }
}

fn find_ident(code: &str, ident: &str, start: usize) -> Option<usize> {
    let mut offset = start;
    while let Some(relative) = code.get(offset..)?.find(ident) {
        let index = offset + relative;
        if ident_boundary(code, index, index + ident.len()) {
            return Some(index);
        }
        offset = index + ident.len();
    }
    None
}

fn ident_end(code: &str, start: usize) -> Option<usize> {
    let bytes = code.as_bytes();
    if !is_ident_start(*bytes.get(start)?) {
        return None;
    }
    let mut end = start + 1;
    while bytes.get(end).is_some_and(|byte| is_ident_continue(*byte)) {
        end += 1;
    }
    Some(end)
}

fn ident_boundary(code: &str, start: usize, end: usize) -> bool {
    let bytes = code.as_bytes();
    !bytes
        .get(start.wrapping_sub(1))
        .is_some_and(|byte| is_ident_continue(*byte))
        && !bytes.get(end).is_some_and(|byte| is_ident_continue(*byte))
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$'
}

fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}
