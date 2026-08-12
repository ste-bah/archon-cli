use archon_llm::provider::{LlmProvider, LlmRequest};

pub(crate) fn apply_system_cache(
    request: &mut LlmRequest,
    provider: &dyn LlmProvider,
    enabled: bool,
    mode: &str,
    ttl: &str,
) {
    if !provider.supports_anthropic_message_caching()
        || !enabled
        || !matches!(mode, "explicit" | "hybrid")
    {
        for block in &mut request.system {
            if let Some(object) = block.as_object_mut() {
                object.remove("cache_control");
            }
        }
        return;
    }
    let Some(block) = request
        .system
        .last_mut()
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    let mut marker = serde_json::json!({"type": "ephemeral"});
    if ttl == "1h" {
        marker["ttl"] = serde_json::json!("1h");
    }
    block.insert("cache_control".into(), marker);
}

pub(crate) fn apply_conversation_cache(
    request: &mut LlmRequest,
    provider: &dyn LlmProvider,
    enabled: bool,
    mode: &str,
    ttl: &str,
) {
    if !provider.supports_anthropic_message_caching() {
        remove_cache_directives(request);
        return;
    }
    if !enabled || !matches!(mode, "explicit" | "hybrid") {
        return;
    }
    let Some(block) = latest_cacheable_block(&mut request.messages) else {
        return;
    };
    let mut marker = serde_json::json!({"type": "ephemeral"});
    if ttl == "1h" {
        marker["ttl"] = serde_json::json!("1h");
    }
    block.insert("cache_control".into(), marker);
}

fn remove_cache_directives(request: &mut LlmRequest) {
    // #171 part 3: `tools` is a shared frozen list, so only take the
    // copy-on-write path when a marker is actually present. Tool schemas
    // built from the registry never carry one, which is why the shared list
    // survives untouched on every non-Anthropic round.
    if request
        .tools
        .iter()
        .any(|tool| tool.get("cache_control").is_some())
    {
        for tool in std::sync::Arc::make_mut(&mut request.tools) {
            if let Some(object) = tool.as_object_mut() {
                object.remove("cache_control");
            }
        }
    }
    for block in &mut request.system {
        if let Some(object) = block.as_object_mut() {
            object.remove("cache_control");
        }
    }
    for message in &mut request.messages {
        let Some(blocks) = message
            .get_mut("content")
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        for block in blocks {
            if let Some(object) = block.as_object_mut() {
                object.remove("cache_control");
            }
        }
    }
}

fn latest_cacheable_block(
    messages: &mut [serde_json::Value],
) -> Option<&mut serde_json::Map<String, serde_json::Value>> {
    for message in messages.iter_mut().rev() {
        let Some(content) = message.get_mut("content") else {
            continue;
        };
        match content {
            serde_json::Value::Array(blocks) => {
                for block in blocks.iter_mut().rev() {
                    let Some(object) = block.as_object_mut() else {
                        continue;
                    };
                    if matches!(
                        object.get("type").and_then(|value| value.as_str()),
                        Some("text" | "tool_result")
                    ) && !object.contains_key("cache_control")
                    {
                        return Some(object);
                    }
                }
            }
            serde_json::Value::String(text) => {
                let text = std::mem::take(text);
                *content = serde_json::json!([{"type":"text","text":text}]);
                return content.as_array_mut()?.last_mut()?.as_object_mut();
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use archon_llm::anthropic::AnthropicClient;
    use archon_llm::auth::AuthProvider;
    use archon_llm::identity::{IdentityMode, IdentityProvider};
    use archon_llm::provider::{
        LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
    };
    use archon_llm::providers::AnthropicProvider;
    use archon_llm::streaming::StreamEvent;
    use archon_llm::types::Secret;

    use super::{apply_conversation_cache, apply_system_cache};

    fn provider(api_url: Option<&str>) -> AnthropicProvider {
        let identity = IdentityProvider::new(
            IdentityMode::Clean,
            "session".into(),
            "device".into(),
            String::new(),
        );
        AnthropicProvider::new(AnthropicClient::new(
            AuthProvider::ApiKey(Secret::new("test-key".into())),
            identity,
            api_url.map(str::to_string),
        ))
    }

    struct NativeCachingProvider;

    #[async_trait::async_trait]
    impl LlmProvider for NativeCachingProvider {
        fn name(&self) -> &str {
            "bedrock"
        }

        fn models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }

        fn supports_feature(&self, feature: ProviderFeature) -> bool {
            feature == ProviderFeature::PromptCaching
        }

        async fn stream(
            &self,
            _: LlmRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
            unreachable!()
        }

        async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
            unreachable!()
        }
    }

    fn request() -> LlmRequest {
        LlmRequest {
            system: vec![serde_json::json!({"type":"text","text":"system"})],
            messages: vec![
                serde_json::json!({"role":"user","content":"first"}),
                serde_json::json!({"role":"assistant","content":[{"type":"text","text":"reply"}]}),
                serde_json::json!({"role":"user","content":[{"type":"text","text":"latest"}]}),
            ],
            tools: archon_llm::provider::shared_tools(vec![
                serde_json::json!({"name":"Read","cache_control":{"type":"ephemeral"}}),
            ]),
            ..LlmRequest::default()
        }
    }

    #[test]
    fn official_anthropic_marks_stable_system_boundary_when_enabled() {
        let direct = provider(None);
        let mut request = request();
        request
            .system
            .push(serde_json::json!({"type":"text","text":"stable workflow universe"}));

        apply_system_cache(&mut request, &direct, true, "explicit", "5m");

        assert_eq!(request.system[1]["cache_control"]["type"], "ephemeral");
        assert_eq!(request.system[0].get("cache_control"), None);
    }

    #[test]
    fn disabled_or_unsupported_system_cache_removes_markers() {
        let direct = provider(None);
        let proxy = provider(Some("http://localhost:11434/v1/messages"));
        for (provider, enabled, mode) in [
            (&direct as &dyn LlmProvider, false, "explicit"),
            (&direct as &dyn LlmProvider, true, "automatic"),
            (&proxy as &dyn LlmProvider, true, "explicit"),
        ] {
            let mut request = request();
            request.system[0]["cache_control"] = serde_json::json!({"type":"ephemeral"});

            apply_system_cache(&mut request, provider, enabled, mode, "5m");

            assert_eq!(request.system[0].get("cache_control"), None);
        }
    }

    #[test]
    fn anthropic_marks_latest_conversation_block_without_mutating_source() {
        let original = request();
        let mut projected = original.clone();

        let direct = provider(None);
        apply_conversation_cache(&mut projected, &direct, true, "explicit", "5m");

        assert_eq!(
            original.messages[2]["content"][0].get("cache_control"),
            None
        );
        assert_eq!(
            projected.messages[2]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert_eq!(projected.messages[2]["content"][0]["text"], "latest");
    }

    #[test]
    fn anthropic_compatible_proxy_does_not_receive_conversation_marker() {
        let mut request = request();
        request.system[0]["cache_control"] = serde_json::json!({"type":"ephemeral"});
        let proxy = provider(Some("http://localhost:11434/v1/messages"));

        apply_conversation_cache(&mut request, &proxy, true, "explicit", "5m");

        assert_eq!(request.system[0].get("cache_control"), None);
        assert_eq!(request.messages[2]["content"][0].get("cache_control"), None);
        assert_eq!(request.tools[0].get("cache_control"), None);
    }

    #[test]
    fn native_prompt_caching_provider_does_not_receive_anthropic_marker() {
        let mut request = request();

        apply_conversation_cache(&mut request, &NativeCachingProvider, true, "explicit", "5m");

        assert_eq!(request.messages[2]["content"][0].get("cache_control"), None);
    }

    #[test]
    fn unsupported_or_disabled_modes_do_not_mark_conversation() {
        let direct = provider(None);
        let proxy = provider(Some("http://localhost:11434/v1/messages"));
        for (provider, enabled, mode) in [
            (&proxy as &dyn LlmProvider, true, "explicit"),
            (&direct as &dyn LlmProvider, false, "explicit"),
            (&direct as &dyn LlmProvider, true, "automatic"),
        ] {
            let mut request = request();
            apply_conversation_cache(&mut request, provider, enabled, mode, "5m");
            assert_eq!(request.messages[2]["content"][0].get("cache_control"), None);
        }
    }

    #[test]
    fn conversation_marker_is_added_before_anthropic_wire_budgeting() {
        let mut request = request();
        request.system = (0..3)
            .map(|index| {
                serde_json::json!({
                    "type":"text",
                    "text":format!("system-{index}"),
                    "cache_control":{"type":"ephemeral"}
                })
            })
            .collect();

        let direct = provider(None);
        apply_conversation_cache(&mut request, &direct, true, "explicit", "5m");

        assert_eq!(
            request.messages[2]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn marks_latest_tool_result_block_for_incremental_tool_round_caching() {
        let mut request = request();
        request.messages.push(serde_json::json!({
            "role":"user",
            "content":[{
                "type":"tool_result",
                "tool_use_id":"tool-1",
                "content":"result"
            }]
        }));

        let direct = provider(None);
        apply_conversation_cache(&mut request, &direct, true, "explicit", "5m");

        assert_eq!(
            request.messages[3]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert_eq!(request.messages[2]["content"][0].get("cache_control"), None);
    }

    #[test]
    fn skips_contentless_messages_when_finding_latest_cacheable_block() {
        let mut request = request();
        request
            .messages
            .push(serde_json::json!({"role":"assistant"}));

        let direct = provider(None);
        apply_conversation_cache(&mut request, &direct, true, "explicit", "5m");

        assert_eq!(
            request.messages[2]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn one_hour_ttl_is_applied_to_conversation_marker() {
        let mut request = request();

        let direct = provider(None);
        apply_conversation_cache(&mut request, &direct, true, "hybrid", "1h");

        assert_eq!(
            request.messages[2]["content"][0]["cache_control"]["ttl"],
            "1h"
        );
    }
}
