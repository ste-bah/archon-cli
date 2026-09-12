use super::*;
use archon_llm::provider::{LlmError, LlmRequest, LlmResponse, ModelInfo, ProviderFeature};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::ContentBlockType;
use archon_tools::tool::Tool;
use serde_json::json;

struct Provider {
    requests: std::sync::Mutex<Vec<LlmRequest>>,
    message_to_lead: bool,
}
#[async_trait]
impl LlmProvider for Provider {
    fn name(&self) -> &str { "delivery-fixture" }
    fn models(&self) -> Vec<ModelInfo> { vec![] }
    fn supports_feature(&self, _: ProviderFeature) -> bool { false }
    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> { unreachable!() }
    async fn stream(&self, request: LlmRequest) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let first = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len() == 1
        };
        let mut events = vec![StreamEvent::MessageStart {
            id: "fixture".into(), model: "fixture".into(), usage: Default::default(),
        }];
        if first && self.message_to_lead {
            events.extend([
                StreamEvent::ContentBlockStart { index: 0, block_type: ContentBlockType::ToolUse,
                    tool_use_id: Some("send".into()), tool_name: Some("SendMessage".into()) },
                StreamEvent::InputJsonDelta { index: 0, partial_json: json!({
                    "to":"lead", "message":"child progress", "summary":"report progress"
                }).to_string() },
            ]);
        } else {
            events.extend([
                StreamEvent::ContentBlockStart { index: 0, block_type: ContentBlockType::Text,
                    tool_use_id: None, tool_name: None },
                StreamEvent::TextDelta { index: 0, text: report() },
            ]);
        }
        events.extend([StreamEvent::ContentBlockStop { index: 0 }, StreamEvent::MessageStop]);
        let (tx, rx) = tokio::sync::mpsc::channel(events.len());
        for event in events { tx.send(event).await.unwrap(); }
        Ok(rx)
    }
}
fn report() -> String { format!("{}FINAL_FINDING", "evidence ".repeat(80)) }
fn request(name: &str) -> SubagentRequest {
    serde_json::from_value(json!({"prompt":"inspect", "subagent_type":name,
        "max_turns":3, "timeout_secs":30, "allowed_tools":["SendMessage"]})).unwrap()
}
fn fixture(message_to_lead: bool) -> (tempfile::TempDir, AgentSubagentExecutor, Arc<Provider>) {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join(".archon/agents");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("reviewer.md"),
        "---\nname: reviewer\ndescription: fixture reviewer\ntools: Read\n---\nSPECIALIST_PROMPT").unwrap();
    let provider = Arc::new(Provider { requests: Default::default(), message_to_lead });
    let agents = AgentRegistry::load_with_user_home(temp.path(), None);
    let executor = AgentSubagentExecutor::new(
        provider.clone(), crate::dispatch::create_default_registry(temp.path().into(), None),
        Arc::new(Mutex::new(SubagentManager::new(4))), Arc::new(std::sync::RwLock::new(agents)),
        None, None, temp.path().into(), uuid::Uuid::new_v4().to_string(), "fixture".into(),
        vec![], Arc::new(Mutex::new("bypassPermissions".into())),
        Arc::new(Mutex::new(HashMap::new())), Arc::new(crate::agent::AgentConfig::default()),
        Arc::new(IdentityProvider::new(archon_llm::identity::IdentityMode::Clean,
            "fixture".into(), String::new(), String::new())),
    );
    (temp, executor, provider)
}
#[tokio::test]
async fn delivery_executor_installs_discovery_on_actual_tool_registry() {
    let (_temp, executor, _) = fixture(false);
    let tool = executor.tool_registry.get("Agent").unwrap();
    assert!(tool.description().contains("reviewer"));
    let catalog = executor.tool_registry.get("AgentCatalog").expect("catalog installed");
    let result = catalog.execute(json!({"action":"info","name":"reviewer"}), &ToolContext::default()).await;
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("fixture reviewer"));
}
#[tokio::test]
async fn delivery_unknown_specialist_fails_before_provider_call() {
    let (_temp, executor, provider) = fixture(false);
    let result = executor.run_to_completion("unknown-child".into(), request("not-a-real-agent"),
        ToolContext::default(), CancellationToken::new()).await;
    assert!(result.is_err(), "unknown specialist must not run generically");
    assert!(result.unwrap_err().to_string().contains("not-a-real-agent"));
    assert!(provider.requests.lock().unwrap().is_empty());
}
#[tokio::test]
async fn delivery_actual_runner_reports_full_result_and_progress_to_parent() {
    let (temp, executor, provider) = fixture(true);
    let ctx = ToolContext { working_dir: temp.path().into(), subagent_id: Some("parent".into()),
        session_id: executor.session_id.clone(), ..Default::default() };
    let result = executor.run_to_completion("child".into(), request("reviewer"), ctx,
        CancellationToken::new()).await.unwrap();
    assert_eq!(result, report());
    let mut manager = executor.subagent_manager.lock().await;
    let messages = manager.drain_pending_messages("parent");
    assert!(messages.iter().any(|m| m == "child progress"), "{messages:?}");
    assert!(messages.iter().any(|m| m.contains(&report())), "full result must reach parent: {messages:?}");
    assert!(manager.drain_pending_messages(crate::message_router::LEAD_QUEUE_ID).is_empty());
    let requests = provider.requests.lock().unwrap();
    assert!(serde_json::to_string(&requests[0].system).unwrap().contains("SPECIALIST_PROMPT"));
    assert!(!serde_json::to_string(&requests[0].tools).unwrap().contains("\"name\":\"Agent\""));
}
#[tokio::test]
async fn delivery_top_level_still_receives_full_result() {
    let (temp, executor, _) = fixture(false);
    executor.run_to_completion("top-child".into(), request("reviewer"),
        ToolContext { working_dir: temp.path().into(), ..Default::default() },
        CancellationToken::new()).await.unwrap();
    let messages = executor.subagent_manager.lock().await
        .drain_pending_messages(crate::message_router::LEAD_QUEUE_ID);
    assert!(messages.iter().any(|m| m.contains(&report())));
}
#[tokio::test]
async fn delivery_result_lookup_uses_retained_name_without_resuming() {
    let (_temp, executor, provider) = fixture(false);
    let mut manager = executor.subagent_manager.lock().await;
    manager.register_with_id("finished-child".into(), request("reviewer")).unwrap();
    manager.complete("finished-child", report()).unwrap();
    manager.cleanup_agent("finished-child");
    drop(manager);
    let tool = archon_tools::send_message::SendMessageTool;
    let input = json!({"to":"reviewer", "message_type":"result"});
    let envelope = tool.execute(input, &ToolContext::default()).await;
    assert!(!envelope.is_error, "{}", envelope.content);
    let context = crate::message_router::RouterContext::new(executor.subagent_manager.clone(),
        crate::message_router::SenderIdentity::Lead);
    let host = NoResume;
    let result = crate::message_router::maybe_route_send_message(&context, &host, "SendMessage", envelope).await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&result.content).unwrap()["result"], report());
    assert!(provider.requests.lock().unwrap().is_empty());
}

struct NoResume;
#[async_trait]
impl crate::message_router::RouterHost for NoResume {
    async fn on_delivered(&self, _: &str, _: &str) {}
}

#[tokio::test]
async fn delivery_failed_specialist_reports_error_to_parent() {
    let (_temp, executor, _) = fixture(false);
    let ctx = ToolContext { subagent_id: Some("parent".into()), ..Default::default() };
    assert!(executor.run_to_completion("failed-child".into(), request("unknown-type"), ctx,
        CancellationToken::new()).await.is_err());
    let mut manager = executor.subagent_manager.lock().await;
    let messages = manager.drain_pending_messages("parent");
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("status=\"failed\""));
    assert!(messages[0].contains("unknown-type"));
    assert!(manager.drain_pending_messages(crate::message_router::LEAD_QUEUE_ID).is_empty());
}

#[tokio::test]
async fn delivery_result_lookup_reports_running_and_failed_without_restart() {
    let (_temp, executor, provider) = fixture(false);
    executor.subagent_manager.lock().await
        .register_with_id("pending".into(), request("reviewer")).unwrap();
    let context = crate::message_router::RouterContext::new(executor.subagent_manager.clone(),
        crate::message_router::SenderIdentity::Lead);
    for status in ["running", "failed"] {
        if status == "failed" {
            executor.subagent_manager.lock().await.mark_failed("pending", "failure evidence".into()).unwrap();
        }
        let envelope = archon_tools::send_message::SendMessageTool.execute(
            json!({"to":"pending", "message_type":"result"}), &ToolContext::default()).await;
        let result = crate::message_router::maybe_route_send_message(&context, &NoResume,
            "SendMessage", envelope).await;
        assert!(!result.is_error, "{}", result.content);
        let data: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(data["status"], status);
        if status == "failed" { assert_eq!(data["error"], "failure evidence"); }
    }
    assert!(provider.requests.lock().unwrap().is_empty());
}
