use super::*;

struct SlowRetryClient;
#[async_trait::async_trait]
impl WorkflowLlmClient for SlowRetryClient {
    /// Scripted replies stand for one continued session (#241).
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.run_agent(call).await
    }

    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!()
    }
    async fn run_agent(
        &self,
        _: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        // Each attempt takes its slot at once, as the executor reports it.
        use archon_tools::subagent_dispatch_clock::{admitted, current_call, scope_session};
        let clocks = current_call().into_iter().collect();
        scope_session("slow-author", clocks, async {
            assert!(admitted("slow-author"));
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        })
        .await;
        Err(archon_workflow::WorkflowError::StageFailed(
            "connection reset by peer".into(),
        ))
    }
}

#[tokio::test]
async fn raw_author_deadline_covers_all_transient_retries() {
    let (sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(SlowRetryClient),
        sink,
        vec![],
        "test".into(),
        None,
        Some(1),
    )
    .with_fixed_raw_tool_policy(vec!["Read".into()]);
    let start = std::time::Instant::now();
    let error = client
        .run_agent_raw_request(&request(WorkflowV2HostMethod::Agent, None), "author".into())
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("author attempt deadline exceeded after 1s"),
        "{error}"
    );
    assert!(start.elapsed() < std::time::Duration::from_millis(1800));
}

/// Waits for a subagent slot far longer than the deadline, reporting the wait
/// the way the executor does, then runs briefly inside it.
struct QueuedClient;
#[async_trait::async_trait]
impl WorkflowLlmClient for QueuedClient {
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.run_agent(call).await
    }

    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!()
    }
    async fn run_agent(
        &self,
        _: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        use archon_tools::subagent_dispatch_clock::{
            admitted, current_call, scope_session, slot_wait,
        };
        let clocks = current_call().into_iter().collect();
        scope_session("queued-author", clocks, async {
            let paused = slot_wait("queued-author");
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            assert!(admitted("queued-author"));
            drop(paused);
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        })
        .await;
        Ok(WorkflowAgentOutcome {
            content: "authored".into(),
            tool_uses: Vec::new(),
            tokens_in: 0,
            tokens_out: 0,
            stop_reason: Some("end_turn".into()),
        })
    }
}

/// Issue 288: a slot wait does not count against the author attempt
/// deadline; the deadline starts when the call starts to run.
#[tokio::test(start_paused = true)]
async fn raw_author_deadline_does_not_count_a_slot_wait() {
    let (sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(QueuedClient),
        sink,
        vec![],
        "test".into(),
        None,
        Some(1),
    )
    .with_fixed_raw_tool_policy(vec!["Read".into()]);
    let outcome = client
        .run_agent_raw_request(&request(WorkflowV2HostMethod::Agent, None), "author".into())
        .await
        .expect("a minute in the queue is not run time");
    assert_eq!(outcome.content, "authored");
}
