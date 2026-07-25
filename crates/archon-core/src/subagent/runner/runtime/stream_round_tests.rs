use std::sync::{Arc, Mutex};

use super::projected_request;
use crate::subagent::runner::SubagentRunner;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature};
use archon_llm::streaming::StreamEvent;
use archon_tools::tool::ToolContext;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

struct AnthropicTestProvider;

struct StalledProvider {
    started: Mutex<Option<oneshot::Sender<()>>>,
    dropped: Mutex<Option<oneshot::Sender<()>>>,
}

#[async_trait::async_trait]
impl LlmProvider for StalledProvider {
    fn name(&self) -> &str {
        "stalled"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        _: LlmRequest,
    ) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        let (tx, rx) = mpsc::channel(1);
        let dropped = self.dropped.lock().unwrap().take().unwrap();
        tokio::spawn(async move {
            tx.closed().await;
            let _ = dropped.send(());
        });
        self.started.lock().unwrap().take().unwrap().send(()).ok();
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("stalled provider only streams")
    }
}

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
        Arc::new(test_identity()),
    )
}

fn test_identity() -> IdentityProvider {
    IdentityProvider::new(
        IdentityMode::Clean,
        "session".into(),
        "device".into(),
        String::new(),
    )
}

#[tokio::test]
async fn cancellation_drops_stalled_provider_stream_promptly() {
    let (started_tx, started_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let cancel = CancellationToken::new();
    let stalled = SubagentRunner::new(
        Arc::new(StalledProvider {
            started: Mutex::new(Some(started_tx)),
            dropped: Mutex::new(Some(dropped_tx)),
        }),
        String::new(),
        Vec::new(),
        Arc::new(crate::dispatch::ToolRegistry::new()),
        ToolContext {
            cancel_parent: Some(cancel.clone()),
            ..ToolContext::default()
        },
        "stalled-model".into(),
        1,
        60,
        Arc::new(crate::agent::AgentConfig::default()),
        Arc::new(test_identity()),
    );
    let run = tokio::spawn(async move { stalled.run("wait forever").await });
    tokio::time::timeout(std::time::Duration::from_secs(1), started_rx)
        .await
        .expect("provider stream should start")
        .expect("start signal should be sent");

    cancel.cancel();

    let result = tokio::time::timeout(std::time::Duration::from_secs(1), run)
        .await
        .expect("runner should observe cancellation")
        .expect("runner task should not panic")
        .expect_err("cancelled inference should not succeed");
    assert!(
        result
            .to_string()
            .contains("Subagent cancelled during LLM inference")
    );
    tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
        .await
        .expect("provider receiver should be dropped")
        .expect("drop signal should be sent");
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
