use super::*;

#[async_trait::async_trait]
impl LlmClient for InvalidItemsThenRepairAgentClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        anyhow::bail!("test should use run_agent");
    }

    async fn run_agent(&self, request: AgentExecutionRequest) -> Result<LlmResponse> {
        self.requests.lock().expect("requests lock").push(request);
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let content = if call == 0 {
            "Context was restored. What would you like to do next?".to_string()
        } else {
            r#"{"items":[{"id":"T001","task":"Implement T001","evidence":"source inspection found missing T001","target_files":["src/lib.rs"]}]}"#.to_string()
        };
        Ok(LlmResponse {
            content,
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for BlockedInvalidItemsAgentClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        unreachable!("test uses run_agent")
    }

    async fn run_agent(&self, _request: AgentExecutionRequest) -> Result<LlmResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        self.release.notified().await;
        Ok(LlmResponse {
            content: "Context was restored. What would you like to do next?".to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for AlwaysInvalidItemsAgentClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        anyhow::bail!("test should use run_agent");
    }

    async fn run_agent(&self, _request: AgentExecutionRequest) -> Result<LlmResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LlmResponse {
            content: "Context was restored. What would you like to do next?".to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}
