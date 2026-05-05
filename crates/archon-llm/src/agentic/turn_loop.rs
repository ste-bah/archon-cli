use async_trait::async_trait;

use super::{
    AgenticLlmProvider, AgenticToolCall, AgenticToolResult, AgenticTurnOutcome, AgenticTurnRequest,
    TurnEventSink, append_tool_results,
};
use crate::provider::LlmError;

#[derive(Debug, Clone, Copy)]
pub struct AgenticLoopConfig {
    pub max_tool_turns: usize,
}

impl Default for AgenticLoopConfig {
    fn default() -> Self {
        Self { max_tool_turns: 8 }
    }
}

#[async_trait]
pub trait AgenticToolExecutor: Send + Sync {
    async fn execute(&self, call: &AgenticToolCall) -> Result<AgenticToolResult, LlmError>;
}

pub async fn run_agentic_tool_loop(
    provider: &dyn AgenticLlmProvider,
    mut request: AgenticTurnRequest,
    executor: &dyn AgenticToolExecutor,
    sink: TurnEventSink,
    config: AgenticLoopConfig,
) -> Result<AgenticTurnOutcome, LlmError> {
    let max_turns = config.max_tool_turns.max(1);
    for _ in 0..max_turns {
        let outcome = provider.stream_turn(request.clone(), sink.clone()).await?;
        if outcome.tool_calls.is_empty() {
            return Ok(outcome);
        }
        persist_tool_exchange(&mut request, &outcome);
        request.tool_results = execute_tools(&outcome.tool_calls, executor).await?;
    }
    Err(LlmError::Unsupported(format!(
        "agentic loop exceeded {max_turns} tool turns"
    )))
}

async fn execute_tools(
    calls: &[AgenticToolCall],
    executor: &dyn AgenticToolExecutor,
) -> Result<Vec<AgenticToolResult>, LlmError> {
    let mut results = Vec::with_capacity(calls.len());
    for call in calls {
        results.push(executor.execute(call).await?);
    }
    Ok(results)
}

fn persist_tool_exchange(request: &mut AgenticTurnRequest, outcome: &AgenticTurnOutcome) {
    let previous_results = std::mem::take(&mut request.tool_results);
    append_tool_results(&mut request.messages, &previous_results);
    request.messages.push(serde_json::json!({
        "role": "assistant",
        "content": assistant_blocks(outcome),
    }));
    request.reasoning_encrypted = outcome.reasoning_encrypted.clone();
}

fn assistant_blocks(outcome: &AgenticTurnOutcome) -> Vec<serde_json::Value> {
    let mut blocks = Vec::new();
    if !outcome.text.is_empty() {
        blocks.push(serde_json::json!({
            "type": "text",
            "text": outcome.text,
        }));
    }
    for call in &outcome.tool_calls {
        blocks.push(serde_json::json!({
            "type": "tool_use",
            "id": call.id,
            "name": call.name,
            "input": call.arguments.clone().unwrap_or(serde_json::Value::Null),
        }));
    }
    blocks
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::agentic::{AgenticTurnEvent, ProviderCapabilitySet};
    use crate::providers::ProviderCapability;
    use crate::types::Usage;

    struct FakeAgenticProvider {
        requests: Arc<Mutex<Vec<AgenticTurnRequest>>>,
        outcomes: Mutex<VecDeque<AgenticTurnOutcome>>,
    }

    #[async_trait]
    impl AgenticLlmProvider for FakeAgenticProvider {
        async fn stream_turn(
            &self,
            request: AgenticTurnRequest,
            sink: TurnEventSink,
        ) -> Result<AgenticTurnOutcome, LlmError> {
            self.requests
                .lock()
                .expect("request capture mutex")
                .push(request);
            let outcome = self
                .outcomes
                .lock()
                .expect("outcomes mutex")
                .pop_front()
                .expect("fake outcome");
            sink.emit(AgenticTurnEvent::TurnCompleted {
                stop_reason: outcome.stop_reason.clone(),
            })
            .await?;
            Ok(outcome)
        }

        fn capabilities(&self) -> ProviderCapabilitySet {
            ProviderCapabilitySet::new([ProviderCapability::Streaming, ProviderCapability::ToolUse])
        }

        fn provider_id(&self) -> &str {
            "openai-codex"
        }

        fn model_id(&self) -> &str {
            "gpt-5.4"
        }
    }

    struct EchoToolExecutor;

    #[async_trait]
    impl AgenticToolExecutor for EchoToolExecutor {
        async fn execute(&self, call: &AgenticToolCall) -> Result<AgenticToolResult, LlmError> {
            Ok(AgenticToolResult {
                tool_call_id: call.id.clone(),
                content: serde_json::json!({"echo": call.arguments}),
                is_error: false,
            })
        }
    }

    #[tokio::test]
    async fn tool_loop_continues_after_tool_result_and_returns_final_answer() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider = FakeAgenticProvider {
            requests: requests.clone(),
            outcomes: Mutex::new(VecDeque::from([
                outcome_with_tool_call(),
                outcome_with_final_text(),
            ])),
        };

        let outcome = run_agentic_tool_loop(
            &provider,
            request(),
            &EchoToolExecutor,
            TurnEventSink::discard(),
            AgenticLoopConfig { max_tool_turns: 4 },
        )
        .await
        .expect("tool loop");

        let captured = requests.lock().expect("request capture mutex");
        assert_eq!(outcome.text, "final answer");
        assert_eq!(captured.len(), 2);
        assert!(
            captured[1].messages[1]["content"]
                .as_array()
                .expect("assistant content blocks")
                .iter()
                .any(|block| block["type"] == "tool_use")
        );
        assert_eq!(captured[1].tool_results[0].tool_call_id, "call_1");
    }

    #[tokio::test]
    async fn tool_loop_errors_when_max_turns_is_exceeded() {
        let provider = FakeAgenticProvider {
            requests: Arc::new(Mutex::new(Vec::new())),
            outcomes: Mutex::new(VecDeque::from([
                outcome_with_tool_call(),
                outcome_with_tool_call(),
            ])),
        };

        let err = run_agentic_tool_loop(
            &provider,
            request(),
            &EchoToolExecutor,
            TurnEventSink::discard(),
            AgenticLoopConfig { max_tool_turns: 1 },
        )
        .await
        .expect_err("loop should stop at max turns");

        assert!(err.to_string().contains("exceeded 1 tool turns"));
    }

    fn request() -> AgenticTurnRequest {
        AgenticTurnRequest {
            model: "gpt-5.4".into(),
            max_tokens: 64,
            system: Vec::new(),
            messages: vec![serde_json::json!({
                "role": "user",
                "content": [{"type": "text", "text": "use a tool"}]
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

    fn outcome_with_tool_call() -> AgenticTurnOutcome {
        AgenticTurnOutcome {
            provider_id: "openai-codex".into(),
            model: "gpt-5.4".into(),
            text: "checking".into(),
            tool_calls: vec![AgenticToolCall {
                id: "call_1".into(),
                name: "Lookup".into(),
                arguments_raw: "{\"query\":\"archon\"}".into(),
                arguments: Some(serde_json::json!({"query": "archon"})),
            }],
            usage: Usage::default(),
            stop_reason: Some("tool_calls".into()),
            reasoning_encrypted: Some("encrypted".into()),
        }
    }

    fn outcome_with_final_text() -> AgenticTurnOutcome {
        AgenticTurnOutcome {
            provider_id: "openai-codex".into(),
            model: "gpt-5.4".into(),
            text: "final answer".into(),
            tool_calls: Vec::new(),
            usage: Usage::default(),
            stop_reason: Some("completed".into()),
            reasoning_encrypted: None,
        }
    }
}
