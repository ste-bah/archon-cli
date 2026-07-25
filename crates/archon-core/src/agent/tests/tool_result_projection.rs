struct CapturingLlmProvider {
    captured: Arc<std::sync::Mutex<Vec<LlmRequest>>>,
}

#[async_trait::async_trait]
impl LlmProvider for CapturingLlmProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_anthropic_message_caching(&self) -> bool {
        true
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        self.captured.lock().unwrap().push(request);
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(tx);
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn main_anthropic_request_marks_latest_conversation_block() {
    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut config = AgentConfig::default();
    config.context.prompt_cache_conversation = true;
    let mut agent = Agent::new(
        Arc::new(CapturingLlmProvider {
            captured: Arc::clone(&captured),
        }),
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.state.add_user_message("latest turn");

    let prepared = agent
        .prepare_turn_request("latest turn", 0)
        .await
        .expect("prepare request");

    assert_eq!(
        prepared.request.messages[0]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert_eq!(agent.state.messages[0]["content"], "latest turn");
}

#[tokio::test]
async fn main_request_trims_old_tool_result_without_mutating_canonical_history() {
    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut config = AgentConfig::default();
    config.context.preserve_recent_turns = 1;
    let mut agent = Agent::new(
        Arc::new(CapturingLlmProvider {
            captured: Arc::clone(&captured),
        }),
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    let old_content = "old-result".repeat(20_000);
    agent.state.messages = vec![
        serde_json::json!({"role":"user","content":"first turn"}),
        serde_json::json!({"role":"assistant","content":[{
            "type":"tool_use","id":"tool-1","name":"Bash","input":{}
        }]}),
        serde_json::json!({"role":"user","content":[{
            "type":"tool_result","tool_use_id":"tool-1","content":old_content,"is_error":false
        }]}),
        serde_json::json!({"role":"user","content":"latest turn"}),
    ];

    let prepared = agent
        .prepare_turn_request("latest turn", 0)
        .await
        .expect("prepare request");

    let projected = prepared.request.messages[2]["content"][0]["content"]
        .as_str()
        .expect("projected tool result");
    assert!(projected.contains("tool output trimmed"));
    assert_eq!(
        agent.state.messages[2]["content"][0]["content"],
        serde_json::Value::String(old_content)
    );
}

#[tokio::test]
async fn eight_tool_rounds_send_trimmed_history_but_reopen_with_full_results() {
    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut config = AgentConfig::default();
    config.context.preserve_recent_turns = 3;
    let mut agent = Agent::new(
        Arc::new(CapturingLlmProvider {
            captured: Arc::clone(&captured),
        }),
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    let full_results: Vec<String> = (0..8)
        .map(|round| format!("round-{round}-{}", "x".repeat(100_000)))
        .collect();
    for (round, result) in full_results.iter().enumerate() {
        agent
            .state
            .messages
            .push(serde_json::json!({"role":"user","content":format!("turn {round}")}));
        agent.state.messages.push(serde_json::json!({"role":"assistant","content":[{
            "type":"tool_use","id":format!("tool-{round}"),"name":"Bash","input":{}
        }]}));
        agent.state.messages.push(serde_json::json!({"role":"user","content":[{
            "type":"tool_result","tool_use_id":format!("tool-{round}"),
            "content":result,"is_error":false
        }]}));
    }
    agent.state.add_user_message("ninth turn");

    let prepared = agent
        .prepare_turn_request("ninth turn", 0)
        .await
        .expect("prepare request");
    agent
        .client
        .stream(prepared.request)
        .await
        .expect("send captured request");

    let request = captured.lock().unwrap().pop().expect("captured request");
    for round in 0..6 {
        assert!(request.messages[round * 3 + 2]["content"][0]["content"]
            .as_str()
            .expect("old tool result")
            .contains("tool output trimmed"));
    }
    for (round, full) in full_results.iter().enumerate().take(8).skip(6) {
        assert_eq!(
            request.messages[round * 3 + 2]["content"][0]["content"],
            *full,
            "recent round {round} should remain byte-identical",
        );
    }

    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("sessions.db");
    let session_id = {
        let store = archon_session::storage::SessionStore::open(&path).expect("open session store");
        let session = store
            .create_session("/tmp", None, "mock")
            .expect("create session");
        let messages = agent
            .state
            .messages
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .expect("serialize canonical messages");
        store
            .replace_messages(&session.id, &messages)
            .expect("persist canonical messages");
        session.id
    };
    let reopened =
        archon_session::storage::SessionStore::open(&path).expect("reopen session store");
    let persisted = reopened
        .load_messages(&session_id)
        .expect("reload messages");
    for round in 0..8 {
        let message: serde_json::Value =
            serde_json::from_str(&persisted[round * 3 + 2]).expect("deserialize tool result");
        assert_eq!(message["content"][0]["content"], full_results[round]);
    }
}

#[tokio::test]
async fn canonical_tool_result_survives_session_store_reopen_after_request_projection() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut config = AgentConfig::default();
    config.context.preserve_recent_turns = 1;
    let mut agent = Agent::new(
        Arc::new(MockLlmProvider),
        ToolRegistry::new(),
        config,
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    let old_content = "persisted-result".repeat(20_000);
    agent.state.messages = vec![
        serde_json::json!({"role":"user","content":"first turn"}),
        serde_json::json!({"role":"assistant","content":[{
            "type":"tool_use","id":"tool-1","name":"Bash","input":{}
        }]}),
        serde_json::json!({"role":"user","content":[{
            "type":"tool_result","tool_use_id":"tool-1","content":old_content,"is_error":false
        }]}),
        serde_json::json!({"role":"user","content":"latest turn"}),
    ];

    let prepared = agent
        .prepare_turn_request("latest turn", 0)
        .await
        .expect("prepare request");
    assert!(
        prepared.request.messages[2]["content"][0]["content"]
            .as_str()
            .expect("projected tool result")
            .contains("tool output trimmed")
    );

    let temp = tempfile::tempdir().expect("create temp directory");
    let path = temp.path().join("sessions.db");
    let session_id = {
        let store = archon_session::storage::SessionStore::open(&path).expect("open session store");
        let session = store
            .create_session("/tmp", None, "mock")
            .expect("create session");
        let messages = agent
            .state
            .messages
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .expect("serialize canonical messages");
        store
            .replace_messages(&session.id, &messages)
            .expect("persist canonical messages");
        session.id
    };

    let reopened =
        archon_session::storage::SessionStore::open(&path).expect("reopen session store");
    let persisted = reopened
        .load_messages(&session_id)
        .expect("reload messages");
    let persisted_tool_result: serde_json::Value =
        serde_json::from_str(&persisted[2]).expect("deserialize tool result");
    assert_eq!(
        persisted_tool_result["content"][0]["content"],
        serde_json::Value::String(old_content)
    );
}
