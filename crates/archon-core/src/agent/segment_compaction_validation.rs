pub fn validate_compaction_source(messages: &[serde_json::Value]) -> Result<(), String> {
    for (index, message) in messages.iter().enumerate() {
        let role = message
            .get("role")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("message {index} has no role"))?;
        if !matches!(role, "user" | "assistant") {
            return Err(format!("message {index} has unsupported role {role}"));
        }
        let content = message
            .get("content")
            .ok_or_else(|| format!("message {index} has no content"))?;
        if !content.is_string() && !content.is_array() {
            return Err(format!("message {index} has invalid content"));
        }
        for block in content.as_array().into_iter().flatten() {
            validate_source_block(index, role, block)?;
        }
    }
    validate_tool_pairs(messages)
}

fn validate_source_block(
    index: usize,
    role: &str,
    block: &serde_json::Value,
) -> Result<(), String> {
    let kind = block
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("message {index} has an untyped content block"))?;
    match kind {
        "text" if string_field_missing(block, "text") => {
            Err(format!("message {index} has invalid text block"))
        }
        "thinking" if string_field_missing(block, "thinking") => {
            Err(format!("message {index} has invalid thinking block"))
        }
        "image" | "image_url" | "input_image" | "document" | "file" | "input_file"
            if block.get("source").is_none() && block.get("url").is_none() =>
        {
            Err(format!("message {index} has invalid {kind} block"))
        }
        "tool_use"
            if role != "assistant"
                || string_field_missing(block, "id")
                || string_field_missing(block, "name")
                || block.get("input").is_none() =>
        {
            Err(format!("message {index} has invalid tool use"))
        }
        "tool_result"
            if role != "user"
                || string_field_missing(block, "tool_use_id")
                || block.get("content").is_none() =>
        {
            Err(format!("message {index} has invalid tool result"))
        }
        "text" | "thinking" | "image" | "image_url" | "input_image" | "document" | "file"
        | "input_file" | "tool_use" | "tool_result" => Ok(()),
        _ => Err(format!("message {index} has unsupported block type {kind}")),
    }
}

fn validate_tool_pairs(messages: &[serde_json::Value]) -> Result<(), String> {
    for (index, message) in messages.iter().enumerate() {
        let tool_ids = block_ids(message, "tool_use", "id");
        if has_duplicates(&tool_ids) {
            return Err(format!("message {index} has duplicate tool use IDs"));
        }
        if !tool_ids.is_empty()
            && !messages
                .get(index + 1)
                .is_some_and(|result| result_ids_match(result, &tool_ids))
        {
            return Err(format!("message {index} has unmatched tool use"));
        }
        let result_ids = block_ids(message, "tool_result", "tool_use_id");
        if has_duplicates(&result_ids)
            || (!result_ids.is_empty()
                && (index == 0
                    || !result_ids_match(
                        message,
                        &block_ids(&messages[index - 1], "tool_use", "id"),
                    )))
        {
            return Err(format!("message {index} has orphaned tool result"));
        }
    }
    Ok(())
}

fn string_field_missing(value: &serde_json::Value, field: &str) -> bool {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .is_none()
}

fn block_ids<'a>(message: &'a serde_json::Value, kind: &str, field: &str) -> Vec<&'a str> {
    message
        .get("content")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(serde_json::Value::as_str) == Some(kind))
        .filter_map(|block| block.get(field).and_then(serde_json::Value::as_str))
        .collect()
}

fn has_duplicates(values: &[&str]) -> bool {
    let unique: std::collections::HashSet<_> = values.iter().collect();
    unique.len() != values.len()
}

fn result_ids_match(result: &serde_json::Value, expected: &[&str]) -> bool {
    let actual = block_ids(result, "tool_result", "tool_use_id");
    !expected.is_empty()
        && expected.len() == actual.len()
        && expected.iter().all(|id| actual.contains(id))
}
