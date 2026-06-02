use serde_json::Value;

pub(crate) fn append_delta_by_index<T, F>(
    items: &mut [T],
    indices: &[u32],
    index: u32,
    partial_json: &str,
    mut append: F,
) -> bool
where
    F: FnMut(&mut T, &str),
{
    let Some(pos) = indices.iter().rposition(|seen| *seen == index) else {
        return false;
    };
    let Some(item) = items.get_mut(pos) else {
        return false;
    };
    append(item, partial_json);
    true
}

pub(crate) fn parse_pending_tool_input(
    tool_name: &str,
    tool_id: &str,
    raw: &str,
    allow_empty: bool,
) -> Result<Value, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        if allow_empty {
            return Ok(serde_json::json!({}));
        }
        return Err(format!(
            "Tool '{tool_name}' ({tool_id}) produced no JSON input; \
             the stream did not deliver input_json_delta for this tool call"
        ));
    }

    serde_json::from_str(trimmed).map_err(|err| {
        let mut message = format!(
            "Tool '{tool_name}' ({tool_id}) produced malformed JSON input \
             at line {}, column {}: {err}. Input preview: {}",
            err.line(),
            err.column(),
            input_preview(trimmed),
        );
        if tool_name == "Write" {
            message.push_str(
                ". Recovery hint: do not retry a large full-file Write. \
                 For existing large files, use LargeEditBegin plus \
                 LargeEditReplaceSection/LargeEditInsertAfter/LargeEditDeleteSection, \
                 then LargeEditCommit.",
            );
        }
        message
    })
}

pub(crate) fn schema_allows_empty_input(schema: &Value) -> bool {
    schema
        .get("required")
        .and_then(Value::as_array)
        .is_none_or(|required| required.is_empty())
}

fn input_preview(raw: &str) -> String {
    let mut preview = String::new();
    for ch in raw.chars().take(240) {
        match ch {
            '\n' | '\r' | '\t' => preview.push(' '),
            c if c.is_control() => preview.push(' '),
            c => preview.push(c),
        }
    }
    if raw.chars().count() > 240 {
        preview.push_str("...");
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, PartialEq, Eq)]
    struct ToolBuf(String);

    #[test]
    fn appends_delta_to_matching_stream_index() {
        let mut tools = vec![ToolBuf::default(), ToolBuf::default(), ToolBuf::default()];
        let indices = vec![4, 9, 12];

        assert!(append_delta_by_index(
            &mut tools,
            &indices,
            9,
            r#"{"file_path":"out.md"}"#,
            |tool, delta| tool.0.push_str(delta),
        ));

        assert_eq!(tools[0].0, "");
        assert_eq!(tools[1].0, r#"{"file_path":"out.md"}"#);
        assert_eq!(tools[2].0, "");
    }

    #[test]
    fn rejects_empty_tool_input() {
        let err = parse_pending_tool_input("Write", "tool-1", " ", false).unwrap_err();
        assert!(err.contains("produced no JSON input"));
    }

    #[test]
    fn accepts_empty_tool_input_when_schema_allows_it() {
        let input = parse_pending_tool_input("DocList", "tool-1", " ", true).unwrap();
        assert_eq!(input, serde_json::json!({}));
    }

    #[test]
    fn rejects_malformed_tool_input_with_preview() {
        let err = parse_pending_tool_input("Write", "tool-1", r#"{"file_path":"x.md""#, false)
            .unwrap_err();
        assert!(err.contains("malformed JSON input"));
        assert!(err.contains("file_path"));
        assert!(err.contains("LargeEditBegin"));
    }

    #[test]
    fn detects_schemas_without_required_fields_as_empty_safe() {
        assert!(schema_allows_empty_input(&serde_json::json!({
            "type": "object",
            "properties": {}
        })));
        assert!(schema_allows_empty_input(&serde_json::json!({
            "type": "object",
            "required": []
        })));
        assert!(!schema_allows_empty_input(&serde_json::json!({
            "type": "object",
            "required": ["file_path"]
        })));
    }
}
