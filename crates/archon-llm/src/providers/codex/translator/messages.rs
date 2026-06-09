use std::collections::{HashMap, HashSet};

use crate::provider::{LlmError, LlmRequest};
use crate::providers::codex::types::{ResponseContentBlock, ResponseInputItem};

pub fn join_system_prompt(system: &[serde_json::Value]) -> Option<String> {
    let mut parts = Vec::new();
    for block in system {
        if block.get("role").and_then(|v| v.as_str()) == Some("system") {
            tracing::warn!("OpenAI role/content system block is not valid Archon internal input");
            continue;
        }
        if block.get("type").and_then(|v| v.as_str()) != Some("text") {
            tracing::warn!("Codex translator ignored non-text system block");
            continue;
        }
        if let Some(text) = block.get("text").and_then(|v| v.as_str())
            && !text.is_empty()
        {
            parts.push(text.to_string());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

pub fn messages_to_responses_input(req: &LlmRequest) -> Result<Vec<ResponseInputItem>, LlmError> {
    let mut input = Vec::new();

    for message in &req.messages {
        let role = message
            .get("role")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LlmError::Serialize("message missing role".into()))?;
        let content = message
            .get("content")
            .ok_or_else(|| LlmError::Serialize("message missing content".into()))?;
        let blocks = content_blocks(content);
        let mut response_blocks = Vec::new();

        for block in blocks {
            match block.get("type").and_then(|v| v.as_str()) {
                Some("text") => push_text(role, block, &mut response_blocks),
                Some("image") => push_image(block, &mut response_blocks),
                Some("tool_use") if role == "assistant" => {
                    flush_message(role, &mut response_blocks, &mut input);
                    input.push(tool_use_to_function_call(block)?);
                }
                Some("tool_result") => {
                    flush_message(role, &mut response_blocks, &mut input);
                    input.push(tool_result_to_output(block));
                }
                Some("thinking") => tracing::trace!("dropping thinking block for Codex"),
                Some(other) => {
                    tracing::warn!("Codex translator skipped unsupported block: {other}")
                }
                None if block.is_string() => push_text(role, block, &mut response_blocks),
                None => tracing::warn!("Codex translator skipped content block without type"),
            }
        }

        flush_message(role, &mut response_blocks, &mut input);
    }

    let mut input = repair_responses_tool_pairs(input);
    if let (Some(index), Some(blob)) = (
        last_assistant_index(&input),
        req.reasoning_encrypted.clone(),
    ) {
        input.insert(
            index,
            ResponseInputItem::Reasoning {
                encrypted_content: blob,
                summary: Vec::new(),
            },
        );
    }

    Ok(input)
}

fn last_assistant_index(input: &[ResponseInputItem]) -> Option<usize> {
    input.iter().rposition(
        |item| matches!(item, ResponseInputItem::Message { role, .. } if role == "assistant"),
    )
}

fn repair_responses_tool_pairs(input: Vec<ResponseInputItem>) -> Vec<ResponseInputItem> {
    let mut output = Vec::with_capacity(input.len());
    let mut used_outputs = HashSet::new();
    let mut i = 0;

    while i < input.len() {
        if !matches!(input.get(i), Some(ResponseInputItem::FunctionCall { .. })) {
            push_non_call_item(&mut output, &input[i]);
            i += 1;
            continue;
        }

        let mut calls = Vec::new();
        while let Some(ResponseInputItem::FunctionCall { call_id, name, .. }) = input.get(i) {
            if call_id.is_empty() {
                output.push(function_call_as_message(&input[i]));
            } else {
                output.push(input[i].clone());
                calls.push((call_id.clone(), name.clone()));
            }
            i += 1;
        }

        let call_ids: HashSet<String> = calls.iter().map(|(id, _)| id.clone()).collect();
        let mut matched = HashMap::new();
        let mut remainder = Vec::new();

        while i < input.len() {
            if matches!(input.get(i), Some(ResponseInputItem::FunctionCall { .. })) {
                break;
            }
            match &input[i] {
                ResponseInputItem::FunctionCallOutput { call_id, .. }
                    if call_ids.contains(call_id)
                        && !matched.contains_key(call_id)
                        && !used_outputs.contains(call_id) =>
                {
                    used_outputs.insert(call_id.clone());
                    matched.insert(call_id.clone(), input[i].clone());
                }
                ResponseInputItem::FunctionCallOutput { .. } => {
                    remainder.push(function_call_output_as_message(&input[i]));
                }
                other => remainder.push(other.clone()),
            }
            i += 1;
        }

        for (call_id, _name) in calls {
            output.push(
                matched
                    .remove(&call_id)
                    .unwrap_or_else(|| missing_function_call_output(call_id)),
            );
        }
        output.extend(remainder);
    }

    output
}

fn push_non_call_item(output: &mut Vec<ResponseInputItem>, item: &ResponseInputItem) {
    match item {
        ResponseInputItem::FunctionCallOutput { .. } => {
            output.push(function_call_output_as_message(item));
        }
        _ => output.push(item.clone()),
    }
}

fn missing_function_call_output(call_id: String) -> ResponseInputItem {
    ResponseInputItem::FunctionCallOutput {
        call_id,
        output: "aborted".to_string(),
    }
}

fn function_call_output_as_message(item: &ResponseInputItem) -> ResponseInputItem {
    let ResponseInputItem::FunctionCallOutput { call_id, output } = item else {
        return item.clone();
    };
    let id = if call_id.is_empty() {
        "unknown"
    } else {
        call_id.as_str()
    };
    ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ResponseContentBlock::InputText {
            text: format!("[Tool result {id}] {output}"),
        }],
    }
}

fn function_call_as_message(item: &ResponseInputItem) -> ResponseInputItem {
    let ResponseInputItem::FunctionCall {
        name, arguments, ..
    } = item
    else {
        return item.clone();
    };
    ResponseInputItem::Message {
        role: "assistant".to_string(),
        content: vec![ResponseContentBlock::OutputText {
            text: format!("[tool call skipped: {name} missing call_id] {arguments}"),
            logprobs: None,
        }],
    }
}

fn content_blocks(content: &serde_json::Value) -> Vec<&serde_json::Value> {
    match content {
        serde_json::Value::Array(items) => items.iter().collect(),
        serde_json::Value::String(_) => vec![content],
        _ => Vec::new(),
    }
}

fn push_text(
    role: &str,
    block: &serde_json::Value,
    response_blocks: &mut Vec<ResponseContentBlock>,
) {
    let text = block
        .get("text")
        .and_then(|v| v.as_str())
        .or_else(|| block.as_str())
        .unwrap_or_default()
        .to_string();
    if role == "assistant" {
        response_blocks.push(ResponseContentBlock::OutputText {
            text,
            logprobs: None,
        });
    } else {
        response_blocks.push(ResponseContentBlock::InputText { text });
    }
}

fn push_image(block: &serde_json::Value, response_blocks: &mut Vec<ResponseContentBlock>) {
    let Some(source) = block.get("source") else {
        return;
    };
    let image_url = match source.get("type").and_then(|v| v.as_str()) {
        Some("url") => source
            .get("url")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        Some("base64") => {
            let media_type = source.get("media_type").and_then(|v| v.as_str());
            let data = source.get("data").and_then(|v| v.as_str());
            media_type
                .zip(data)
                .map(|(m, d)| format!("data:{m};base64,{d}"))
        }
        _ => None,
    };
    if let Some(image_url) = image_url {
        response_blocks.push(ResponseContentBlock::InputImage {
            image_url,
            detail: None,
        });
    }
}

fn flush_message(
    role: &str,
    response_blocks: &mut Vec<ResponseContentBlock>,
    input: &mut Vec<ResponseInputItem>,
) {
    if response_blocks.is_empty() {
        return;
    }
    input.push(ResponseInputItem::Message {
        role: role.to_string(),
        content: std::mem::take(response_blocks),
    });
}

fn tool_use_to_function_call(block: &serde_json::Value) -> Result<ResponseInputItem, LlmError> {
    let call_id = string_field(block, "id").unwrap_or_default();
    let name = string_field(block, "name").unwrap_or_default();
    let input = block
        .get("input")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let arguments = serde_json::to_string(&input)
        .map_err(|e| LlmError::Serialize(format!("tool_use input: {e}")))?;
    Ok(ResponseInputItem::FunctionCall {
        call_id,
        name,
        arguments,
    })
}

fn tool_result_to_output(block: &serde_json::Value) -> ResponseInputItem {
    let call_id = string_field(block, "tool_use_id").unwrap_or_default();
    let mut output = block
        .get("content")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            block
                .get("content")
                .map(ToString::to_string)
                .unwrap_or_default()
        });
    if block
        .get("is_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        output = format!("[ERROR]: {output}");
    }
    ResponseInputItem::FunctionCallOutput { call_id, output }
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(messages: Vec<serde_json::Value>) -> LlmRequest {
        LlmRequest {
            model: "codex".into(),
            max_tokens: 64,
            system: Vec::new(),
            messages,
            tools: Vec::new(),
            thinking: None,
            speed: None,
            effort: None,
            extra: serde_json::Value::Null,
            request_origin: None,
            reasoning_encrypted: None,
        }
    }

    fn input_json(messages: Vec<serde_json::Value>) -> serde_json::Value {
        serde_json::to_value(messages_to_responses_input(&request(messages)).unwrap()).unwrap()
    }

    #[test]
    fn codex_responses_preserves_valid_tool_pair() {
        let input = input_json(vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{"type": "tool_use", "id": "call_1", "name": "Read", "input": {"path": "a"}}]
            }),
            serde_json::json!({
                "role": "user",
                "content": [{"type": "tool_result", "tool_use_id": "call_1", "content": "ok"}]
            }),
        ]);

        assert_eq!(input[0]["type"], "function_call");
        assert_eq!(input[0]["call_id"], "call_1");
        assert_eq!(input[1]["type"], "function_call_output");
        assert_eq!(input[1]["call_id"], "call_1");
        assert_eq!(input[1]["output"], "ok");
    }

    #[test]
    fn codex_responses_inserts_aborted_output_for_missing_result() {
        let input = input_json(vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{"type": "tool_use", "id": "call_1", "name": "Read", "input": {}}]
            }),
            serde_json::json!({"role": "user", "content": "continue"}),
        ]);

        assert_eq!(input[0]["type"], "function_call");
        assert_eq!(input[1]["type"], "function_call_output");
        assert_eq!(input[1]["call_id"], "call_1");
        assert_eq!(input[1]["output"], "aborted");
        assert_eq!(input[2]["type"], "message");
        assert_eq!(input[2]["role"], "user");
    }

    #[test]
    fn codex_responses_textifies_orphan_tool_result() {
        let input = input_json(vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "tool_result", "tool_use_id": "call_orphan", "content": "late"}]
        })]);

        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert!(
            input[0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("call_orphan")
        );
    }

    #[test]
    fn codex_responses_moves_late_matching_output_before_user_text() {
        let input = input_json(vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{"type": "tool_use", "id": "call_1", "name": "Read", "input": {}}]
            }),
            serde_json::json!({"role": "user", "content": "intervening"}),
            serde_json::json!({
                "role": "user",
                "content": [{"type": "tool_result", "tool_use_id": "call_1", "content": "ok"}]
            }),
        ]);

        assert_eq!(input[0]["type"], "function_call");
        assert_eq!(input[1]["type"], "function_call_output");
        assert_eq!(input[1]["output"], "ok");
        assert_eq!(input[2]["type"], "message");
        assert_eq!(input[2]["content"][0]["text"], "intervening");
    }

    #[test]
    fn codex_responses_handles_parallel_calls_in_call_order() {
        let input = input_json(vec![
            serde_json::json!({
                "role": "assistant",
                "content": [
                    {"type": "tool_use", "id": "call_1", "name": "Read", "input": {}},
                    {"type": "tool_use", "id": "call_2", "name": "Write", "input": {}}
                ]
            }),
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "call_2", "content": "two"},
                    {"type": "tool_result", "tool_use_id": "call_1", "content": "one"}
                ]
            }),
        ]);

        assert_eq!(input[0]["type"], "function_call");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(input[2]["output"], "one");
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_2");
        assert_eq!(input[3]["output"], "two");
    }

    #[test]
    fn codex_responses_textifies_duplicate_tool_output() {
        let input = input_json(vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{"type": "tool_use", "id": "call_1", "name": "Read", "input": {}}]
            }),
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "call_1", "content": "first"},
                    {"type": "tool_result", "tool_use_id": "call_1", "content": "second"}
                ]
            }),
        ]);

        assert_eq!(input[1]["type"], "function_call_output");
        assert_eq!(input[1]["output"], "first");
        assert_eq!(input[2]["type"], "message");
        assert!(
            input[2]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("second")
        );
    }
}
