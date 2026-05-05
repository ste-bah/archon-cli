use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::*;
use crate::provider::{LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature};
use crate::streaming::StreamEvent;
use crate::types::{ContentBlockType, Usage};

struct FakeProvider {
    name: String,
    events: Vec<StreamEvent>,
}

#[async_trait]
impl LlmProvider for FakeProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "fake-model".into(),
            display_name: "Fake Model".into(),
            context_window: 1000,
        }]
    }

    async fn stream(&self, _request: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        let (tx, rx) = mpsc::channel(self.events.len().max(1));
        for event in &self.events {
            tx.try_send(event.clone())
                .map_err(|err| LlmError::Http(err.to_string()))?;
        }
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        Err(LlmError::Unsupported("fake complete is not used".into()))
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        matches!(
            feature,
            ProviderFeature::Streaming | ProviderFeature::ToolUse
        )
    }
}

fn adapter(events: Vec<StreamEvent>) -> LlmProviderAgenticAdapter {
    LlmProviderAgenticAdapter::new(
        Arc::new(FakeProvider {
            name: "fake".into(),
            events,
        }),
        "fake",
        "fake-model",
        ProviderCapabilitySet::new([ProviderCapability::Streaming, ProviderCapability::ToolUse]),
    )
}

fn request() -> AgenticTurnRequest {
    AgenticTurnRequest {
        model: "fake-model".into(),
        max_tokens: 64,
        system: vec![serde_json::json!({"type": "text", "text": "system"})],
        messages: vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "hello"}]
        })],
        tools: Vec::new(),
        tool_results: Vec::new(),
        effort: None,
        reasoning_encrypted: None,
        session_id: None,
        run_id: None,
        parent_id: None,
        budget_usd_remaining: None,
        extra: serde_json::Value::Null,
    }
}

#[tokio::test]
async fn adapter_maps_text_usage_and_completion() {
    let provider = adapter(vec![
        StreamEvent::MessageStart {
            id: "msg_1".into(),
            model: "fake-model".into(),
            usage: Usage {
                input_tokens: 3,
                output_tokens: 0,
                ..Usage::default()
            },
        },
        StreamEvent::TextDelta {
            index: 0,
            text: "hello ".into(),
        },
        StreamEvent::TextDelta {
            index: 0,
            text: "world".into(),
        },
        StreamEvent::MessageDelta {
            stop_reason: Some("end_turn".into()),
            usage: Some(Usage {
                input_tokens: 0,
                output_tokens: 2,
                ..Usage::default()
            }),
        },
        StreamEvent::MessageStop,
    ]);
    let (sink, mut events) = TurnEventSink::channel(16);

    let outcome = provider.stream_turn(request(), sink).await.unwrap();
    let mut observed = Vec::new();
    while let Some(event) = events.recv().await {
        observed.push(event);
    }

    assert_eq!(outcome.provider_id, "fake");
    assert_eq!(outcome.model, "fake-model");
    assert_eq!(outcome.text, "hello world");
    assert_eq!(outcome.usage.input_tokens, 3);
    assert_eq!(outcome.usage.output_tokens, 2);
    assert_eq!(outcome.stop_reason.as_deref(), Some("end_turn"));
    assert!(
        observed
            .iter()
            .any(|event| matches!(event, AgenticTurnEvent::TextDelta { text } if text == "hello "))
    );
    assert!(matches!(
        observed.last(),
        Some(AgenticTurnEvent::TurnCompleted { .. })
    ));
}

#[tokio::test]
async fn adapter_maps_tool_call_lifecycle() {
    let provider = adapter(vec![
        StreamEvent::MessageStart {
            id: "msg_1".into(),
            model: "fake-model".into(),
            usage: Usage::default(),
        },
        StreamEvent::ContentBlockStart {
            index: 0,
            block_type: ContentBlockType::ToolUse,
            tool_use_id: Some("toolu_1".into()),
            tool_name: Some("Lookup".into()),
        },
        StreamEvent::InputJsonDelta {
            index: 0,
            partial_json: "{\"query\":\"arc".into(),
        },
        StreamEvent::InputJsonDelta {
            index: 0,
            partial_json: "hon\"}".into(),
        },
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::MessageStop,
    ]);
    let (sink, mut events) = TurnEventSink::channel(16);

    let outcome = provider.stream_turn(request(), sink).await.unwrap();
    let mut observed = Vec::new();
    while let Some(event) = events.recv().await {
        observed.push(event);
    }

    assert_eq!(outcome.tool_calls.len(), 1);
    let call = &outcome.tool_calls[0];
    assert_eq!(call.id, "toolu_1");
    assert_eq!(call.name, "Lookup");
    assert_eq!(call.arguments_raw, "{\"query\":\"archon\"}");
    assert_eq!(call.arguments, Some(serde_json::json!({"query": "archon"})));
    assert!(observed.iter().any(|event| {
        matches!(
            event,
            AgenticTurnEvent::ToolCallStarted { id, name }
                if id == "toolu_1" && name == "Lookup"
        )
    }));
    assert!(observed.iter().any(|event| {
        matches!(
            event,
            AgenticTurnEvent::ToolCallCompleted { call } if call.id == "toolu_1"
        )
    }));
}

#[test]
fn capability_set_reports_supported_features() {
    let set =
        ProviderCapabilitySet::new([ProviderCapability::Streaming, ProviderCapability::ToolUse]);

    assert!(set.supports(ProviderCapability::Streaming));
    assert!(set.supports(ProviderCapability::ToolUse));
    assert!(!set.supports(ProviderCapability::Subagents));
}
