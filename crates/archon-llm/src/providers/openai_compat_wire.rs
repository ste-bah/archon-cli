use super::*;

impl OpenAiCompatProvider {
    /// Build the OpenAI `/v1/chat/completions` request body from a generic
    /// `LlmRequest`. Deliberately minimal: tools, thinking, speed, effort,
    /// and arbitrary `extra` are not forwarded. Explicit temperature is
    /// forwarded for sampling-controlled calls.
    pub(super) fn to_openai_wire(&self, req: &LlmRequest) -> Value {
        let model = if req.model.is_empty() {
            self.descriptor.default_model.clone()
        } else {
            req.model.clone()
        };

        // Merge `req.system` (if any) as a leading system message. Each
        // system entry is already a JSON value; we concatenate their text
        // representations into a single synthetic system message so the
        // wire format remains OpenAI-canonical.
        let mut messages: Vec<Value> = Vec::with_capacity(req.messages.len() + 1);
        if !req.system.is_empty() {
            let system_text = req
                .system
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    // Anthropic-style content block `{"type":"text","text":"..."}`
                    Value::Object(_) => v
                        .get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| v.to_string()),
                    _ => v.to_string(),
                })
                .collect::<Vec<_>>()
                .join("\n");
            messages.push(json!({"role": "system", "content": system_text}));
        }
        for m in &req.messages {
            messages.push(m.clone());
        }

        // TASK-AGS-705: tool_calls serialization branches on
        // `descriptor.quirks.tool_call_format`. Request-side tool
        // forwarding lands in a later slice; the enum is staged here
        // so TASK-AGS-707/708 can consume it without touching request
        // construction. Reading the field prevents dead-code warnings
        // and proves the quirks dispatch path is wired.
        let _tool_format: ToolCallFormat = self.descriptor.quirks.tool_call_format;

        let mut body = json!({
            "model": model,
            "messages": messages,
            "max_tokens": req.max_tokens,
        });
        if let Some(temperature) = req.extra.get("temperature") {
            body["temperature"] = temperature.clone();
        }
        body
    }

    pub(super) fn parse_chat_response(body: Value) -> Result<LlmResponse, LlmError> {
        let choices = body
            .get("choices")
            .and_then(|c| c.as_array())
            .ok_or_else(|| LlmError::Serialize("missing `choices` array in response".into()))?;
        let first = choices
            .first()
            .ok_or_else(|| LlmError::Serialize("`choices` array was empty".into()))?;

        let content = first
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| {
                LlmError::Serialize("missing `choices[0].message.content` string".into())
            })?
            .to_string();

        let stop_reason = first
            .get("finish_reason")
            .and_then(|f| f.as_str())
            .unwrap_or("stop")
            .to_string();

        let usage_json = body.get("usage");
        let input_tokens = usage_json
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output_tokens = usage_json
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        Ok(LlmResponse {
            content: vec![json!({"type": "text", "text": content})],
            usage: Usage {
                input_tokens,
                output_tokens,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                input_tokens_available: usage_json
                    .and_then(|u| u.get("prompt_tokens"))
                    .and_then(|v| v.as_u64())
                    .is_some(),
                output_tokens_available: usage_json
                    .and_then(|u| u.get("completion_tokens"))
                    .and_then(|v| v.as_u64())
                    .is_some(),
                cache_creation_input_tokens_available: false,
                cache_read_input_tokens_available: false,
            },
            stop_reason,
        })
    }
}
