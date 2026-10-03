//! One completion request preparation path for sending and cache identity.
use super::*;

pub(super) fn message_request(adapter: &ProviderLlmAdapter, request: LlmRequest) -> LlmRequest {
    LlmRequest {
        model: adapter.model_for_provider(&request.model),
        max_tokens: adapter.max_tokens,
        system: request.system,
        messages: request.messages,
        tools: request.tools,
        request_origin: adapter.request_origin.clone(),
        extra: request.extra,
        ..LlmRequest::default()
    }
}
