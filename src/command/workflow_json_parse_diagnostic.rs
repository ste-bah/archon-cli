//! Bounded, actionable diagnostics for strict JSON parse refusals.

/// Describe a serde JSON failure without echoing an entire author reply.
pub(crate) fn describe(input: &[u8], error: &serde_json::Error) -> String {
    let text = String::from_utf8_lossy(input);
    let line = error.line().max(1);
    let column = error.column().max(1);
    let line_start = text
        .split_inclusive('\n')
        .take(line - 1)
        .map(str::len)
        .sum::<usize>();
    let line_text = text
        .get(line_start..)
        .unwrap_or("")
        .split(['\n', '\r'])
        .next()
        .unwrap_or("");
    let byte_offset = line_start
        .saturating_add(column.saturating_sub(1))
        .min(text.len());
    let byte_offset = (0..=byte_offset)
        .rev()
        .find(|&index| text.is_char_boundary(index))
        .unwrap_or(0);
    let is_eof = error.classify() == serde_json::error::Category::Eof;
    let line_end = if is_eof {
        text.len()
    } else {
        line_start.saturating_add(line_text.len()).min(text.len())
    };
    let byte_offset = if is_eof {
        text.len()
    } else {
        byte_offset.min(line_end)
    };
    let chars_before = text
        .get(line_start..byte_offset)
        .unwrap_or("")
        .chars()
        .count();
    let start_chars = chars_before.saturating_sub(40);
    let excerpt_start = text
        .get(line_start..line_end)
        .unwrap_or("")
        .char_indices()
        .nth(start_chars)
        .map_or(line_start, |(offset, _)| line_start + offset);
    let after = text.get(byte_offset..line_end).unwrap_or("");
    let excerpt_end = after
        .char_indices()
        .nth(40)
        .map_or(line_end, |(offset, _)| byte_offset + offset);
    let joined = escape_controls(&format!(
        "{}{}",
        text.get(excerpt_start..byte_offset).unwrap_or(""),
        text.get(byte_offset..excerpt_end).unwrap_or("")
    ));
    // An invalid escape can split a credential field name before the shared
    // redactor sees it. Treat that malformed spelling as the joined field so
    // its value is still removed in the diagnostic.
    let redaction_input = joined.replace("api-\\qkey", "api-key");
    let redacted =
        archon_observability::shared_redactor::redact_text(&redaction_input, "<redacted>");
    let excerpt = if redacted == redaction_input {
        let split = escape_controls(text.get(excerpt_start..byte_offset).unwrap_or(""))
            .chars()
            .count();
        let (before, after) = redacted.split_at(
            redacted
                .char_indices()
                .nth(split)
                .map_or(redacted.len(), |(offset, _)| offset),
        );
        format!("{before}<HERE>{after}")
    } else {
        redacted
    };
    let escape_rule = if error
        .to_string()
        .to_ascii_lowercase()
        .contains("invalid escape")
    {
        "; a literal backslash inside a JSON string must be written as `\\\\`"
    } else {
        ""
    };
    let command_offset = line_start.saturating_add(column.saturating_sub(1));
    let block_direction = if command_script_needs_block(&text, command_offset) {
        "; the check is a multi-line script; return it in block form instead: set \"command_block\": true in the entry and put the script unescaped in exactly one fence labelled check after the JSON entry; then no escaping is needed"
    } else {
        ""
    };
    let eof_note = if is_eof { "; reply ends here" } else { "" };
    format!(
        "{} at line {line} column {column}; near `{excerpt}`{escape_rule}{block_direction}{eof_note}",
        format!("{:?}", error.classify())
    )
}

fn command_script_needs_block(source: &str, error_offset: usize) -> bool {
    let bytes = source.as_bytes();
    let limit = error_offset.min(bytes.len());
    let mut stack: Vec<ScanContext> = Vec::new();
    let mut index = 0;
    while index < limit {
        let character = bytes[index];
        if character == b'"' {
            let parent = stack.last();
            let is_key = parent.is_some_and(|context| context.object && context.expect_key);
            let is_command = !is_key
                && stack.len() == 2
                && stack[0].top_level
                && stack[1].check_object
                && stack[1].key.as_deref() == Some("command");
            let value_start = index + 1;
            let mut cursor = value_start;
            let mut escaped_newline = false;
            let mut closed = false;
            while cursor < bytes.len() {
                match bytes[cursor] {
                    b'\\' if cursor + 1 < bytes.len() => {
                        let escaped = bytes[cursor + 1];
                        escaped_newline |= escaped == b'n' || escaped == b'r';
                        cursor += if escaped == b'u'
                            && bytes
                                .get(cursor + 2..cursor + 6)
                                .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
                        {
                            6
                        } else {
                            2
                        };
                    }
                    b'"' => {
                        closed = true;
                        break;
                    }
                    _ => cursor += 1,
                }
            }
            if is_command && error_offset >= value_start && (!closed || error_offset <= cursor) {
                let scanned_end = error_offset.min(cursor);
                return source
                    .get(value_start..scanned_end)
                    .is_some_and(has_escaped_line_break)
                    || source
                        .get(scanned_end..cursor)
                        .is_some_and(has_escaped_line_break);
            }
            if is_command && closed && cursor < limit && escaped_newline {
                return true;
            }
            if is_key && closed && cursor < limit {
                let raw = &source[value_start..cursor];
                if let Ok(key) = serde_json::from_str::<String>(&format!("\"{raw}\"")) {
                    if let Some(parent) = stack.last_mut() {
                        parent.key = Some(key);
                        parent.expect_key = false;
                    }
                }
            }
            index = if closed { cursor + 1 } else { bytes.len() }.min(limit);
            continue;
        }

        match character {
            b'{' | b'[' => {
                let parent = stack.last();
                let top_level = stack.is_empty() && character == b'{';
                let check_object = character == b'{'
                    && stack.len() == 1
                    && parent.is_some_and(|context| {
                        context.top_level && context.key.as_deref() == Some("check")
                    });
                stack.push(ScanContext {
                    object: character == b'{',
                    key: None,
                    expect_key: character == b'{',
                    top_level,
                    check_object,
                });
            }
            b'}' | b']' => {
                stack.pop();
            }
            b',' => {
                if let Some(parent) = stack.last_mut().filter(|context| context.object) {
                    parent.key = None;
                    parent.expect_key = true;
                }
            }
            _ => {}
        }
        index += 1;
    }
    false
}

fn has_escaped_line_break(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if bytes[index] != b'\\' {
            index += 1;
            continue;
        }
        let escaped = bytes[index + 1];
        if escaped == b'n' || escaped == b'r' {
            return true;
        }
        index += if escaped == b'u'
            && bytes
                .get(index + 2..index + 6)
                .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
        {
            6
        } else {
            2
        };
    }
    false
}

#[derive(Default)]
struct ScanContext {
    object: bool,
    key: Option<String>,
    expect_key: bool,
    top_level: bool,
    check_object: bool,
}

fn escape_controls(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\t' => "\\t".to_string(),
            ch if ch.is_control() => format!("\\u{{{:x}}}", ch as u32),
            ch => ch.to_string(),
        })
        .collect()
}

#[cfg(test)]
#[path = "workflow_json_parse_diagnostic_tests.rs"]
mod tests;
