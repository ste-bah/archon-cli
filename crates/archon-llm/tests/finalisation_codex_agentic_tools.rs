use std::sync::{Arc, Mutex};

use archon_llm::agentic::{
    AgenticLlmProvider, AgenticToolResult, AgenticTurnRequest, LlmProviderAgenticAdapter,
    ProviderCapabilitySet, TurnEventSink,
};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::providers::ProviderCapability;
use archon_llm::providers::codex::translator::{
    messages_to_responses_input, process_responses_stream, tools_to_responses_tools,
};
use archon_llm::providers::codex::types::{ResponseInputItem, ResponseStreamEvent};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::{ContentBlockType, Usage};
use async_trait::async_trait;
use tokio::sync::mpsc;

#[test]
fn codex_tool_schema_conversion_preserves_required_nested_and_enum_fields() {
    let tools = vec![serde_json::json!({
        "name": "SearchDocs",
        "description": "Search indexed documents",
        "input_schema": {
            "type": "object",
            "required": ["query", "mode"],
            "properties": {
                "query": {"type": "string"},
                "mode": {"type": "string", "enum": ["exact", "semantic", "hybrid"]},
                "filters": {
                    "type": "object",
                    "properties": {
                        "source": {"type": "string"}
                    }
                }
            }
        }
    })];

    let mapped = tools_to_responses_tools(&tools).expect("schema conversion");

    assert_eq!(mapped[0].kind, "function");
    assert_eq!(mapped[0].name, "SearchDocs");
    assert_eq!(mapped[0].parameters["required"][0], "query");
    assert_eq!(mapped[0].parameters["required"][1], "mode");
    assert_eq!(
        mapped[0].parameters["properties"]["mode"]["enum"][2],
        "hybrid"
    );
    assert_eq!(
        mapped[0].parameters["properties"]["filters"]["properties"]["source"]["type"],
        "string"
    );
}

#[test]
fn codex_stream_parser_emits_multiple_tool_calls_and_done_arguments_once() {
    let events = codex_events([
        serde_json::json!({
            "type":"response.output_item.added",
            "output_index":0,
            "item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"lookup","arguments":"","status":"in_progress"}
        }),
        serde_json::json!({
            "type":"response.function_call_arguments.done",
            "item_id":"fc_1",
            "output_index":0,
            "arguments":"{\"query\":\"alpha\"}"
        }),
        serde_json::json!({
            "type":"response.output_item.added",
            "output_index":1,
            "item":{"type":"function_call","id":"fc_2","call_id":"call_2","name":"read_file","arguments":"","status":"in_progress"}
        }),
        serde_json::json!({
            "type":"response.function_call_arguments.delta",
            "item_id":"fc_2",
            "output_index":1,
            "delta":"{\"path\":\"README.md\"}"
        }),
        serde_json::json!({
            "type":"response.function_call_arguments.done",
            "item_id":"fc_2",
            "output_index":1,
            "arguments":"{\"path\":\"README.md\"}"
        }),
    ]);

    let translated = process_responses_stream(events)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("stream translation");
    let starts = translated
        .iter()
        .filter(|event| matches_tool_start(event))
        .count();
    let argument_deltas = translated
        .iter()
        .filter(|event| matches!(event, StreamEvent::InputJsonDelta { .. }))
        .count();
    let stops = translated
        .iter()
        .filter(|event| matches!(event, StreamEvent::ContentBlockStop { .. }))
        .count();

    assert_eq!(starts, 2);
    assert_eq!(argument_deltas, 2);
    assert_eq!(stops, 2);
}

#[test]
fn codex_tool_result_continuation_serializes_function_output() {
    let mut turn = agentic_request();
    turn.tool_results = vec![AgenticToolResult {
        tool_call_id: "call_1".into(),
        content: serde_json::json!({"answer": 42}),
        is_error: false,
    }];

    let input = messages_to_responses_input(&turn.into_llm_request()).expect("codex input");

    assert!(input.iter().any(|item| {
        matches!(
            item,
            ResponseInputItem::FunctionCallOutput { call_id, output }
                if call_id == "call_1" && output.contains("\"answer\":42")
        )
    }));
}

#[tokio::test]
async fn agentic_adapter_passes_tool_results_to_underlying_provider() {
    let captured = Arc::new(Mutex::new(None));
    let provider = CaptureProvider {
        captured: captured.clone(),
        events: vec![
            StreamEvent::MessageStart {
                id: "msg_1".into(),
                model: "gpt-5.4".into(),
                usage: Usage::default(),
            },
            StreamEvent::TextDelta {
                index: 0,
                text: "done".into(),
            },
            StreamEvent::MessageStop,
        ],
    };
    let adapter = LlmProviderAgenticAdapter::new(
        Arc::new(provider),
        "openai-codex",
        "gpt-5.4",
        ProviderCapabilitySet::new([ProviderCapability::Streaming, ProviderCapability::ToolUse]),
    );
    let mut request = agentic_request();
    request.tool_results = vec![AgenticToolResult {
        tool_call_id: "call_1".into(),
        content: serde_json::json!("ok"),
        is_error: false,
    }];

    let outcome = adapter
        .stream_turn(request, TurnEventSink::discard())
        .await
        .expect("agentic turn");
    let seen = captured
        .lock()
        .expect("capture mutex")
        .clone()
        .expect("captured request");
    let input = messages_to_responses_input(&seen).expect("codex input");

    assert_eq!(outcome.text, "done");
    assert!(input.iter().any(|item| {
        matches!(
            item,
            ResponseInputItem::FunctionCallOutput { call_id, output }
                if call_id == "call_1" && output == "ok"
        )
    }));
}

fn matches_tool_start(event: &StreamEvent) -> bool {
    matches!(
        event,
        StreamEvent::ContentBlockStart {
            block_type: ContentBlockType::ToolUse,
            ..
        }
    )
}

fn codex_events<const N: usize>(values: [serde_json::Value; N]) -> Vec<ResponseStreamEvent> {
    values
        .into_iter()
        .map(|value| serde_json::from_value(value).expect("codex stream event"))
        .collect()
}

fn agentic_request() -> AgenticTurnRequest {
    AgenticTurnRequest {
        model: "gpt-5.4".into(),
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

struct CaptureProvider {
    captured: Arc<Mutex<Option<LlmRequest>>>,
    events: Vec<StreamEvent>,
}

#[async_trait]
impl LlmProvider for CaptureProvider {
    fn name(&self) -> &str {
        "openai-codex"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "gpt-5.4".into(),
            display_name: "GPT-5.4".into(),
            context_window: 200_000,
        }]
    }

    async fn stream(&self, request: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        *self.captured.lock().expect("capture mutex") = Some(request);
        let (tx, rx) = mpsc::channel(self.events.len().max(1));
        for event in &self.events {
            tx.try_send(event.clone())
                .map_err(|err| LlmError::Http(err.to_string()))?;
        }
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        Err(LlmError::Unsupported(
            "capture provider complete unused".into(),
        ))
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        matches!(
            feature,
            ProviderFeature::Streaming | ProviderFeature::ToolUse
        )
    }
}
