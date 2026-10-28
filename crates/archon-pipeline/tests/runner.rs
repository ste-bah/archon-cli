//! Tests for TASK-PIPE-A02: PipelineFacade Trait + Shared Runner Loop
//!
//! These tests verify:
//! - A 3-agent stub pipeline executes end-to-end using a mock LLM
//! - Each agent receives fresh/isolated message history (context isolation)
//! - PipelineResult contains results from all agents
//! - Per-agent overhead is < 2 seconds
//! - Skip semantics work correctly
//! - Quality scores are stored in agent results

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use archon_llm::provider::LlmError;
use archon_pipeline::learning::{
    desc::{DescEpisode, DescEpisodeStore},
    gnn::auto_trainer_runtime::query_trajectories_for_training,
    integration::{LearningIntegration, LearningIntegrationConfig},
    patterns::PatternStore,
    reasoning::{ReasoningBank, ReasoningBankConfig, ReasoningBankDeps},
    reflexion::ReflexionInjector,
    schema::initialize_learning_schemas,
};
use archon_pipeline::runner::{
    AgentExecutionRequest, AgentInfo, AgentResult, LlmClient, LlmResponse, NextAgent,
    PipelineFacade, PipelineProgressFacade, PipelineResult, PipelineSession, PipelineType,
    QualityScore, ToolAccessLevel, run_pipeline,
};

// ---------------------------------------------------------------------------
// Mock LLM Client
// ---------------------------------------------------------------------------

/// Records each call's message count to verify context isolation,
/// and returns a canned response.
#[derive(Clone)]
struct MockLlmClient {
    /// Vec of message-vec lengths recorded per call, in order.
    call_message_counts: Arc<Mutex<Vec<usize>>>,
    /// Flattened system context recorded per call, in order.
    system_texts: Arc<Mutex<Vec<String>>>,
    /// The canned content string to return.
    canned_response: String,
}

#[derive(Clone)]
struct DelayedLlmClient {
    delay: Duration,
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct SequencedLlmClient {
    call_count: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct RecordingSequencedLlmClient {
    call_count: Arc<AtomicUsize>,
    system_texts: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone)]
struct AgentPathLlmClient {
    calls: Arc<Mutex<Vec<(String, String, String)>>>,
}

#[derive(Clone)]
struct FlakyAgentLlmClient {
    calls: Arc<AtomicUsize>,
    failures_before_success: usize,
}

struct FlakyWaveLlmClient {
    calls: Arc<AtomicUsize>,
    failures_before_success: usize,
    error: LlmError,
}

impl DelayedLlmClient {
    fn new(delay: Duration) -> Self {
        Self {
            delay,
            active: Arc::new(AtomicUsize::new(0)),
            max_active: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn max_observed(&self) -> usize {
        self.max_active.load(Ordering::SeqCst)
    }
}

impl SequencedLlmClient {
    fn new() -> Self {
        Self {
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn calls(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

impl RecordingSequencedLlmClient {
    fn new() -> Self {
        Self {
            call_count: Arc::new(AtomicUsize::new(0)),
            system_texts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    fn recorded_system_texts(&self) -> Vec<String> {
        self.system_texts.lock().unwrap().clone()
    }
}

impl AgentPathLlmClient {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> Vec<(String, String, String)> {
        self.calls.lock().unwrap().clone()
    }
}

impl FlakyAgentLlmClient {
    fn new(failures_before_success: usize) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            failures_before_success,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl FlakyWaveLlmClient {
    fn new(failures_before_success: usize, error: LlmError) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            failures_before_success,
            error,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl LlmClient for DelayedLlmClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<LlmResponse> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(LlmResponse {
            content: "wave output".into(),
            tool_uses: vec![],
            tokens_in: Some(10),
            tokens_out: Some(5),
            stop_reason: None,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for SequencedLlmClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<LlmResponse> {
        let call = self.call_count.fetch_add(1, Ordering::SeqCst);
        let content = if call < 2 { "retry-me" } else { "retry-ok" };
        Ok(LlmResponse {
            content: content.into(),
            tool_uses: vec![],
            tokens_in: Some(10),
            tokens_out: Some(5),
            stop_reason: None,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for RecordingSequencedLlmClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<LlmResponse> {
        let system_text = system
            .iter()
            .filter_map(|entry| entry.get("text").and_then(|text| text.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        self.system_texts.lock().unwrap().push(system_text);

        let call = self.call_count.fetch_add(1, Ordering::SeqCst);
        let content = if call < 2 { "retry-me" } else { "retry-ok" };
        Ok(LlmResponse {
            content: content.into(),
            tool_uses: vec![],
            tokens_in: Some(10),
            tokens_out: Some(5),
            stop_reason: None,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for AgentPathLlmClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<LlmResponse> {
        anyhow::bail!("runner should call run_agent when an agent-aware client is provided")
    }

    async fn run_agent(&self, request: AgentExecutionRequest) -> anyhow::Result<LlmResponse> {
        self.calls
            .lock()
            .unwrap()
            .push((request.session_id, request.task, request.agent.key));
        Ok(LlmResponse {
            content: "agent path output".into(),
            tool_uses: vec![],
            tokens_in: Some(11),
            tokens_out: Some(7),
            stop_reason: None,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for FlakyAgentLlmClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<LlmResponse> {
        anyhow::bail!("runner should call run_agent for this test")
    }

    async fn run_agent(&self, _request: AgentExecutionRequest) -> anyhow::Result<LlmResponse> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call < self.failures_before_success {
            anyhow::bail!(
                "subagent failed: HTTP error: http_error: HTTP error: error decoding response body"
            );
        }
        Ok(LlmResponse {
            content: "agent path output".into(),
            tool_uses: vec![],
            tokens_in: Some(11),
            tokens_out: Some(7),
            stop_reason: None,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for FlakyWaveLlmClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<LlmResponse> {
        anyhow::bail!("runner should call run_agent for this test")
    }

    async fn run_agent(&self, _request: AgentExecutionRequest) -> anyhow::Result<LlmResponse> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call < self.failures_before_success {
            return Err(anyhow::Error::new(match &self.error {
                LlmError::Overloaded {
                    status,
                    retry_after_secs,
                    message,
                } => LlmError::Overloaded {
                    status: *status,
                    retry_after_secs: *retry_after_secs,
                    message: message.clone(),
                },
                LlmError::Http(message) => LlmError::Http(message.clone()),
                _ => LlmError::Http("unexpected test error".into()),
            }));
        }
        Ok(LlmResponse {
            content: "wave retry output".into(),
            tool_uses: vec![],
            tokens_in: Some(11),
            tokens_out: Some(7),
            stop_reason: None,
        })
    }
}

impl MockLlmClient {
    fn new(canned_response: &str) -> Self {
        Self {
            call_message_counts: Arc::new(Mutex::new(Vec::new())),
            system_texts: Arc::new(Mutex::new(Vec::new())),
            canned_response: canned_response.to_string(),
        }
    }

    fn recorded_message_counts(&self) -> Vec<usize> {
        self.call_message_counts.lock().unwrap().clone()
    }

    fn recorded_system_texts(&self) -> Vec<String> {
        self.system_texts.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl LlmClient for MockLlmClient {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<LlmResponse> {
        // Record the message count for context-isolation verification.
        self.call_message_counts
            .lock()
            .unwrap()
            .push(messages.len());
        let system_text = system
            .iter()
            .filter_map(|entry| entry.get("text").and_then(|text| text.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        self.system_texts.lock().unwrap().push(system_text);

        Ok(LlmResponse {
            content: self.canned_response.clone(),
            tool_uses: vec![],
            tokens_in: Some(100),
            tokens_out: Some(50),
            stop_reason: None,
        })
    }
}

include!("runner_parts/part_1.rs");
include!("runner_parts/part_2.rs");
include!("runner_parts/part_3.rs");
