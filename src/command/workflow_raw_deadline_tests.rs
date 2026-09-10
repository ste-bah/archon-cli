use super::*;

struct SlowRetryClient;
#[async_trait::async_trait]
impl WorkflowLlmClient for SlowRetryClient {
    async fn send_message(&self, _: Vec<serde_json::Value>, _: Vec<serde_json::Value>, _: Vec<serde_json::Value>, _: &str) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> { unreachable!() }
    async fn run_agent(&self, _: WorkflowAgentCall) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        Err(archon_workflow::WorkflowError::StageFailed("connection reset by peer".into()))
    }
}

#[tokio::test]
async fn raw_author_deadline_covers_all_transient_retries() {
    let (sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(Arc::new(SlowRetryClient), sink, vec![], "test".into(), None, Some(1))
        .with_fixed_raw_tool_policy(vec!["Read".into()]);
    let start = std::time::Instant::now();
    let error = client.run_agent_raw_request(&request(WorkflowV2HostMethod::Agent, None), "author".into()).await.unwrap_err();
    assert!(error.to_string().contains("author attempt deadline"), "{error}");
    assert!(start.elapsed() < std::time::Duration::from_millis(1800));
}
