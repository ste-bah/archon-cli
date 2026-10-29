use super::*;

struct SlowRetryClient(std::sync::atomic::AtomicUsize, bool);
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
        scope_session("AC-X-001", clocks, async {
            assert!(admitted("AC-X-001"));
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 6 {
                if self.1 {
                    std::future::pending::<()>().await;
                }
                return Ok(WorkflowAgentOutcome {
                    content: "AC-X-001".into(),
                    ..Default::default()
                });
            }
            Err(archon_workflow::WorkflowError::StageFailed(
                "connection reset by peer".into(),
            ))
        })
        .await
    }
}

#[tokio::test(start_paused = true)]
async fn raw_author_deadline_excludes_outage_time_but_still_cuts_a_stalled_stream() {
    let (sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(SlowRetryClient(
            std::sync::atomic::AtomicUsize::new(0),
            false,
        )),
        sink,
        vec![],
        "AC-X-001".into(),
        None,
        Some(1),
    )
    .with_fixed_raw_tool_policy(vec!["AC-X-001".into()]);
    let start = tokio::time::Instant::now();
    let outcome = client
        .run_agent_raw_request(
            &request(WorkflowV2HostMethod::Agent, None),
            "TASK-X-001".into(),
        )
        .await
        .expect("an outage longer than three no-progress windows must recover");
    assert_eq!(outcome.content, "AC-X-001");
    assert!(start.elapsed() > std::time::Duration::from_secs(3));

    // A provider that stalls on its first attempt still spends its window.
    let stalled = Arc::new(SlowRetryClient(
        std::sync::atomic::AtomicUsize::new(6),
        true,
    ));
    let (sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        stalled.clone(),
        sink,
        vec![],
        "AC-X-001".into(),
        None,
        Some(1),
    )
    .with_fixed_raw_tool_policy(vec!["AC-X-001".into()]);
    let start = tokio::time::Instant::now();
    let error = client
        .run_agent_raw_request(
            &request(WorkflowV2HostMethod::Agent, None),
            "TASK-X-001".into(),
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("author attempt no-progress deadline exceeded after 1s"),
        "{error}"
    );
    assert_eq!(start.elapsed(), std::time::Duration::from_secs(1));
    assert_eq!(stalled.0.load(std::sync::atomic::Ordering::SeqCst), 7);
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
            tokens_in: Some(0),
            tokens_out: Some(0),
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
