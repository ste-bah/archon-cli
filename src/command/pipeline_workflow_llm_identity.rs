//! A representative completion identity from the actual configured client.
//! Fidelity additionally keys each batch by its complete rendered request.

use archon_llm::provider::LlmRequest;
use archon_pipeline::runner::LlmClient;

pub(crate) fn request_identity(client: &dyn LlmClient) -> Option<String> {
    client.message_request_identity(&LlmRequest {
        model: "opus".into(),
        extra: serde_json::json!({"temperature": 0.0}),
        ..LlmRequest::default()
    })
}

#[cfg(test)]
#[path = "pipeline_workflow_llm_identity_tests.rs"]
mod tests;
