use std::sync::Arc;

use super::projected_request;
use crate::subagent::runner::SubagentRunner;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature};
use archon_llm::streaming::StreamEvent;
use archon_tools::tool::ToolContext;

struct AnthropicTestProvider;

#[async_trait::async_trait]
impl LlmProvider for AnthropicTestProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn supports_anthropic_message_caching(&self) -> bool {
        true
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        _: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        unreachable!("projection tests do not open a stream")
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("projection tests do not complete a request")
    }
}

fn runner() -> SubagentRunner {
    let mut config = crate::agent::AgentConfig::default();
    config.context.prompt_cache = true;
    config.context.prompt_cache_conversation = true;
    SubagentRunner::new(
        Arc::new(AnthropicTestProvider),
        String::new(),
        Vec::new(),
        Arc::new(crate::dispatch::ToolRegistry::new()),
        ToolContext::default(),
        "claude-sonnet-4-6".into(),
        1,
        60,
        Arc::new(config),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "session".into(),
            "device".into(),
            String::new(),
        )),
    )
}

#[test]
fn rebuilt_request_marks_latest_text_without_mutating_history() {
    let runner = runner();
    let messages = vec![serde_json::json!({"role":"user","content":"rebuilt text"})];

    let projected = projected_request(&runner, &messages, &LlmRequest::default());

    assert_eq!(
        projected.messages[0]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert_eq!(messages[0]["content"], "rebuilt text");
}

#[test]
fn rebuilt_request_marks_latest_tool_result_without_mutating_history() {
    let runner = runner();
    let messages = vec![
        serde_json::json!({"role":"user","content":"initial"}),
        serde_json::json!({"role":"assistant","content":[{
            "type":"tool_use","id":"tool-1","name":"Read","input":{}
        }]}),
        serde_json::json!({"role":"user","content":[{
            "type":"tool_result","tool_use_id":"tool-1","content":"result"
        }]}),
    ];

    let projected = projected_request(&runner, &messages, &LlmRequest::default());

    assert_eq!(
        projected.messages[2]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert!(
        projected.messages[0]["content"]
            .as_str()
            .is_some_and(|text| text == "initial")
    );
    assert!(messages[2]["content"][0].get("cache_control").is_none());
}
