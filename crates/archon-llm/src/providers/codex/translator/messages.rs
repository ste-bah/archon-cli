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
    let mut last_assistant_index = None;

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
                None => tracing::warn!("Codex translator skipped content block without type"),
            }
        }

        flush_message(role, &mut response_blocks, &mut input);
        if role == "assistant" {
            last_assistant_index = input.iter().rposition(|item| {
                matches!(item, ResponseInputItem::Message { role, .. } if role == "assistant")
            });
        }
    }

    if let (Some(index), Some(blob)) = (last_assistant_index, req.reasoning_encrypted.clone()) {
        input.insert(
            index,
            ResponseInputItem::Reasoning {
                encrypted_content: blob,
            },
        );
    }

    Ok(input)
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
