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
    let eof_note = if is_eof { "; reply ends here" } else { "" };
    format!(
        "{} at line {line} column {column}; near `{excerpt}`{escape_rule}{eof_note}",
        format!("{:?}", error.classify())
    )
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
