/// Detects assistant output that opens with GLM tool-call markup, whether well-formed or not.
///
/// A final answer never opens with tool-call markup; well-formed or not, the model tried to call a tool.
pub(crate) fn starts_with_text_tool_call(text: &str) -> bool {
    extract_text_tool_call_name(text).is_some()
}

/// Extracts the tool name from the text-call prefix recognized above.
pub(crate) fn extract_text_tool_call_name(text: &str) -> Option<String> {
    let Some(name) = text.trim_start().strip_prefix("<tool_call>") else {
        return None;
    };
    let name = name.trim_start();
    let Some(name_len) = name
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        .map(|(index, ch)| index + ch.len_utf8())
        .last()
    else {
        return None;
    };
    (name_len > 0).then(|| name[..name_len].to_owned())
}

#[cfg(test)]
mod tests {
    use super::{extract_text_tool_call_name, starts_with_text_tool_call};

    #[test]
    fn extracts_names_from_recognized_text_call_shapes() {
        for (text, expected) in [
            ("<tool_call>Read</tool_call>", "Read"),
            ("<tool_call>Grep\nquery", "Grep"),
            ("<tool_call>Bash<arg_key>command</arg_key>...", "Bash"),
        ] {
            assert_eq!(extract_text_tool_call_name(text).as_deref(), Some(expected));
        }
        for text in ["garbage", "<tool_call>", "<tool_call>{\"name\":\"Read\"}"] {
            assert_eq!(extract_text_tool_call_name(text), None, "{text:?}");
        }
    }

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

        let malformed_live_shape = concat!(
            "<tool_call>Grep<arg_key>-n</arg_key><arg_value>true</arg_value>",
            "<arg_key>production_eligible</arg_value></tool_call>"
        );
        assert!(
            starts_with_text_tool_call(malformed_live_shape),
            "{malformed_live_shape:?}"
        );
        assert!(starts_with_text_tool_call(
            "<tool_call>Read<arg_key>file_path"
        ));
    }

    #[test]
    fn rejects_prose_code_and_json_before_calls() {
        for text in [
            "I saw <tool_call>Read</tool_call> in the output.",
            "```xml\n<tool_call>Read</tool_call>\n```",
            r#"{"example":"<tool_call>Read</tool_call>"}"#,
            "Here is the call: <tool_call>Read</tool_call>",
        ] {
            assert!(!starts_with_text_tool_call(text), "{text:?}");
        }

        assert!(starts_with_text_tool_call(
            "<tool_call>Read</tool_call> That is the tool I would use."
        ));
    }
}
