use super::*;

impl AnthropicClient {
    pub fn build_request_body(&self, request: &MessageRequest) -> Result<String, ApiError> {
        let mut body = serde_json::json!({
            "model": request.model,
            "max_tokens": request.max_tokens,
            "stream": true,
            "messages": request.messages,
        });

        // Build the system field. When in Spoof mode (e.g. OAuth token), prepend
        // the canonical Claude Code identity blocks (billing header + identity
        // prefix) so the request is recognised as Claude Code traffic. Idempotent:
        // skip prepending if the caller already provided the identity prefix.
        let mut system_blocks = request.system.clone();
        if matches!(
            self.identity.mode,
            crate::identity::IdentityMode::Spoof { .. }
        ) {
            let has_billing = system_blocks.iter().any(|block| {
                block
                    .get("text")
                    .and_then(|text| text.as_str())
                    .is_some_and(|text| text.starts_with("x-anthropic-billing-header:"))
            });
            let has_identity = system_blocks.iter().any(|block| {
                block
                    .get("text")
                    .and_then(|text| text.as_str())
                    .is_some_and(|text| text.starts_with("You are Claude Code,"))
            });
            if !has_billing {
                let first_user_msg = request
                    .messages
                    .first()
                    .and_then(|message| message.get("content"))
                    .and_then(first_text_content)
                    .unwrap_or("");
                if let Some(billing) = self.identity.billing_header(first_user_msg) {
                    system_blocks.insert(
                        0,
                        serde_json::json!({
                            "type": "text",
                            "text": billing,
                            "cache_control": { "type": "ephemeral" }
                        }),
                    );
                }
            }
            if !has_identity {
                let identity_index = system_blocks
                    .iter()
                    .position(|block| {
                        block
                            .get("text")
                            .and_then(|text| text.as_str())
                            .is_some_and(|text| text.starts_with("x-anthropic-billing-header:"))
                    })
                    .map_or(0, |index| index + 1);
                system_blocks.insert(
                    identity_index,
                    serde_json::json!({
                        "type": "text",
                        "text": "You are Claude Code, Anthropic's official CLI for Claude.",
                        "cache_control": { "type": "ephemeral", "scope": "org" }
                    }),
                );
            }
        }
        if !system_blocks.is_empty() {
            body["system"] = serde_json::json!(system_blocks);
        }

        if !request.tools.is_empty() {
            body["tools"] = if crate::anthropic_url::is_official_messages_url(&self.api_url) {
                serde_json::json!(cached_tool_blocks(&request.tools))
            } else {
                serde_json::json!(request.tools.as_ref())
            };
        }

        if let Some(temperature) = request.temperature {
            body["temperature"] = serde_json::json!(temperature);
        }

        if let Some(ref thinking) = request.thinking {
            body["thinking"] = serde_json::json!(thinking);
        }

        if let Some(speed) = effective_speed(request) {
            body["speed"] = serde_json::json!(speed);
        }

        if let Some(effort) = effective_effort(request) {
            body["output_config"] = serde_json::json!({ "effort": effort });
        }

        let metadata = self.identity.metadata();
        if !metadata.as_object().map(|o| o.is_empty()).unwrap_or(true) {
            body["metadata"] = metadata;
        }

        if let Some(anti_dist) = self.identity.anti_distillation_value() {
            body["anti_distillation"] = anti_dist;
        }

        if crate::anthropic_url::is_official_messages_url(&self.api_url) {
            enforce_cache_breakpoint_budget(&mut body);
        } else {
            remove_cache_directives(&mut body);
        }
        serde_json::to_string(&body).map_err(|e| ApiError::SerializeError(format!("{e}")))
    }
}
