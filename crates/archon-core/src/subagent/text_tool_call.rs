/// Detects assistant output that starts with one or more complete GLM text-form tool calls.
///
/// A final answer never opens with a complete tool call; text after it is the model imagining the tool result.
/// Prose, code, or JSON before a call, and an incomplete opening call, do not trigger a retry.
pub(crate) fn starts_with_text_tool_call(text: &str) -> bool {
    if serde_json::from_str::<serde_json::Value>(text.trim()).is_ok() {
        return false;
    }

    let mut remaining = text.trim();
    let mut calls = 0;
    while !remaining.is_empty() {
        let Some(after_call) = parse_call(remaining) else {
            return calls > 0;
        };
        calls += 1;
        remaining = after_call.trim_start();
    }
    calls > 0
}

fn parse_call(text: &str) -> Option<&str> {
    let mut rest = text.strip_prefix("<tool_call>")?.trim_start();
    let name_len = rest
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        .map(|(index, ch)| index + ch.len_utf8())
        .last()?;
    if name_len == 0 {
        return None;
    }
    rest = rest[name_len..].trim_start();

    while rest.starts_with("<arg_key>") {
        let key_end = rest.find("</arg_key>")?;
        let key = &rest["<arg_key>".len()..key_end];
        if key.trim().is_empty() {
            return None;
        }
        rest = rest[key_end + "</arg_key>".len()..].trim_start();
        let value_start = rest.strip_prefix("<arg_value>")?;
        let value_end = value_start.find("</arg_value>")?;
        rest = value_start[value_end + "</arg_value>".len()..].trim_start();
    }
    rest.strip_prefix("</tool_call>")
}

#[cfg(test)]
mod tests {
    use super::starts_with_text_tool_call;

    #[test]
    fn recognizes_tool_call_text_shapes() {
        for text in [
            "<tool_call>Read</tool_call>",
            "<tool_call>Read<arg_key>path</arg_key><arg_value>/tmp/a</arg_value></tool_call>",
            "<tool_call>Read<arg_key>path</arg_key>\n<arg_value>/tmp/a</arg_value>\n</tool_call>\n",
            "<tool_call>Read<arg_key>path</arg_key><arg_value>a</arg_value></tool_call>\n<tool_call>Write</tool_call>",
        ] {
            assert!(starts_with_text_tool_call(text), "{text:?}");
        }

        let live_shape = concat!(
            "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/tmp/a</arg_value>",
            "<arg_key>limit</arg_key><arg_value>50</arg_value></tool_call>",
            "\n<archon-md>Imagined tool output and context...</archon-md>"
        );
        assert!(starts_with_text_tool_call(live_shape), "{live_shape:?}");
    }

    #[test]
    fn rejects_prose_code_json_and_partial_calls() {
        for text in [
            "I saw <tool_call>Read</tool_call> in the output.",
            "```xml\n<tool_call>Read</tool_call>\n```",
            r#"{"example":"<tool_call>Read</tool_call>"}"#,
            "Here is the call: <tool_call>Read</tool_call>",
            "<tool_call>Read<arg_key>path</arg_key></tool_call>",
        ] {
            assert!(!starts_with_text_tool_call(text), "{text:?}");
        }

        assert!(starts_with_text_tool_call(
            "<tool_call>Read</tool_call> That is the tool I would use."
        ));
    }
}
