//! Pipeline-side refusal fixture: live tests must cross the real port adapter.
use archon_pipeline::runner::{AgentExecutionRequest, LlmClient, LlmResponse};
use std::sync::{Arc, Mutex};

/// The executor refuses to continue the first session (it cannot restore it
/// exactly, #241) and continues any later one. Every call answers from
/// `replies` in order. `typed: false` raises the same words as an ordinary
/// error, as a model value quoted into a validation error would.
pub(crate) struct RefusingContinuation {
    replies: Mutex<Vec<String>>,
    pub(crate) calls: Mutex<Vec<(&'static str, String, String)>>,
    typed: bool,
}
impl RefusingContinuation {
    pub(crate) fn new(replies: &[&str]) -> Arc<Self> {
        Self::build(replies, true)
    }
    pub(crate) fn build(replies: &[&str], typed: bool) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.iter().rev().map(|r| r.to_string()).collect()),
            calls: Mutex::new(vec![]),
            typed,
        })
    }
    fn record(&self, kind: &'static str, call: &AgentExecutionRequest) -> String {
        let prompt = serde_json::to_string(&call.messages).unwrap();
        let mut calls = self.calls.lock().unwrap();
        calls.push((kind, call.session_id.clone(), prompt));
        calls[0].1.clone()
    }
    fn reply(&self) -> anyhow::Result<LlmResponse> {
        Ok(LlmResponse {
            content: self.replies.lock().unwrap().pop().expect("a reply"),
            tool_uses: vec![],
            tokens_in: 0,
            tokens_out: 0,
            stop_reason: Some("end_turn".into()),
        })
    }
    pub(crate) fn kinds(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|call| call.0)
            .collect()
    }
    pub(crate) fn session(&self, index: usize) -> String {
        self.calls.lock().unwrap()[index].1.clone()
    }
}
#[async_trait::async_trait]
impl LlmClient for RefusingContinuation {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> anyhow::Result<LlmResponse> {
        panic!("no provider")
    }
    async fn run_agent(&self, call: AgentExecutionRequest) -> anyhow::Result<LlmResponse> {
        self.record("fresh", &call);
        self.reply()
    }
    async fn continue_agent(&self, call: AgentExecutionRequest) -> anyhow::Result<LlmResponse> {
        if self.record("continue", &call) != call.session_id {
            return self.reply();
        }
        let words = format!(
            "cannot continue agent '{}': its confinement is only known to the process that \
             started it; start a new agent",
            call.session_id
        );
        Err(match self.typed {
            true => anyhow::Error::new(archon_tools::subagent_session::ContinuationRefused(words))
                .context("pipeline continuation failed"),
            false => anyhow::anyhow!("{words}"),
        })
    }
}
