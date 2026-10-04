use super::*;
use archon_workflow::WorkflowAgentOutcome;
use std::sync::Mutex;

#[derive(Default)]
struct SessionPort {
    ids: Mutex<Vec<(bool, String)>>,
}
#[async_trait::async_trait]
impl WorkflowLlmClient for SessionPort {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("no provider")
    }
    async fn run_agent(
        &self,
        call: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.ids.lock().unwrap().push((false, call.session_id));
        Ok(outcome("invalid initial answer".into()))
    }
    async fn continue_agent(
        &self,
        call: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.ids.lock().unwrap().push((true, call.session_id));
        let mut result = archon_workflow::WorkflowV2Result::accepted("inspected");
        result
            .evidence
            .push(archon_workflow::WorkflowV2Evidence::new(
                archon_workflow::WorkflowV2EvidenceKind::Inspection,
                "inspected source contents",
            ));
        Ok(outcome(serde_json::to_string(&result).unwrap()))
    }
}
fn outcome(content: String) -> WorkflowAgentOutcome {
    WorkflowAgentOutcome {
        content,
        tool_uses: vec![],
        tokens_in: 0,
        tokens_out: 0,
        stop_reason: Some("end_turn".into()),
    }
}
#[tokio::test]
async fn live_validation_repair_dispatches_continuation_with_isolated_generation() {
    let port = Arc::new(SessionPort::default());
    let (sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        port.clone(),
        sink,
        vec![],
        "repair-run".into(),
        None,
        Some(10),
    );
    let request = tests::request(WorkflowV2HostMethod::Agent, None);
    let adapter = archon_workflow::WorkflowV2AgentAdapter::new();
    super::super::workflow_live_v2_host_dispatch::run_v2_agent_call_with_rejected_output_log(
        &adapter, &client, &request, None,
    )
    .await
    .unwrap();
    super::super::workflow_live_v2_host_dispatch::run_v2_agent_call_with_rejected_output_log(
        &adapter, &client, &request, None,
    )
    .await
    .unwrap();
    let ids = port.ids.lock().unwrap();
    assert_eq!(
        ids.iter().map(|c| c.0).collect::<Vec<_>>(),
        vec![false, true, false, true]
    );
    assert_eq!(ids[0].1, ids[1].1);
    assert_eq!(ids[2].1, ids[3].1);
    assert_ne!(ids[0].1, ids[2].1);
}

#[tokio::test]
async fn preflight_reask_reuses_author_generation_and_original_request() {
    let port = Arc::new(SessionPort::default());
    let (sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        port.clone(),
        sink,
        vec![],
        "author-run".into(),
        None,
        Some(10),
    );
    let mut request = tests::request(WorkflowV2HostMethod::Agent, None);
    request.call.id = "author-workflow-script".into();
    let adapter = archon_workflow::WorkflowV2AgentAdapter::new();
    archon_workflow::v2::repair_session::author_scope(async {
        super::super::workflow_live_v2_host_dispatch::run_v2_agent_call_with_rejected_output_log(
            &adapter, &client, &request, None,
        )
        .await
        .unwrap();
        request.task = "Repair the previous script, no re-exploration".into();
        super::super::workflow_live_v2_host_dispatch::run_v2_agent_call_with_rejected_output_log(
            &adapter, &client, &request, None,
        )
        .await
        .unwrap();
    })
    .await;
    let ids = port.ids.lock().unwrap();
    assert_eq!(
        ids.iter().map(|c| c.0).collect::<Vec<_>>(),
        vec![false, true, true]
    );
    assert!(ids.iter().all(|c| c.1 == ids[0].1));
}

/// The executor refuses a continuation whose stored context it does not hold
/// or cannot run exactly (#241). Fresh calls answer from `replies` in order.
struct RefusingContinuation {
    replies: Mutex<Vec<String>>,
    calls: Mutex<Vec<(&'static str, String, String)>>,
}
impl RefusingContinuation {
    fn new(replies: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.iter().rev().map(|r| r.to_string()).collect()),
            calls: Mutex::new(vec![]),
        })
    }
    fn record(&self, kind: &'static str, call: &WorkflowAgentCall) {
        let prompt = serde_json::to_string(&call.messages).unwrap();
        self.calls
            .lock()
            .unwrap()
            .push((kind, call.session_id.clone(), prompt));
    }
}
#[async_trait::async_trait]
impl WorkflowLlmClient for RefusingContinuation {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("no provider")
    }
    async fn run_agent(
        &self,
        call: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.record("fresh", &call);
        Ok(outcome(
            self.replies.lock().unwrap().pop().expect("a reply"),
        ))
    }
    async fn continue_agent(
        &self,
        call: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.record("continue", &call);
        Err(archon_workflow::WorkflowError::port(anyhow::anyhow!(
            "subagent failed: cannot continue agent '{}': its confinement is only known to the \
             process that started it; start a new agent",
            call.session_id
        )))
    }
}

fn accepted() -> String {
    let mut result = archon_workflow::WorkflowV2Result::accepted("inspected");
    result
        .evidence
        .push(archon_workflow::WorkflowV2Evidence::new(
            archon_workflow::WorkflowV2EvidenceKind::Inspection,
            "inspected source contents",
        ));
    serde_json::to_string(&result).unwrap()
}

/// The client, and the receiver its activity sink needs kept open.
fn live(port: Arc<RefusingContinuation>) -> (LiveV2AgentClient, impl Sized) {
    let (sink, rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(port, sink, vec![], "refused-run".into(), None, Some(10));
    (client, rx)
}

/// A refused repair continuation starts an explicit new agent, in a new
/// session, told the prior attempt's findings; the call then succeeds.
#[tokio::test]
async fn a_refused_repair_starts_a_new_agent_and_the_call_proceeds() {
    let port = RefusingContinuation::new(&["invalid initial answer", &accepted()]);
    let (client, _rx) = live(port.clone());
    let request = tests::request(WorkflowV2HostMethod::Agent, None);
    let adapter = archon_workflow::WorkflowV2AgentAdapter::new();
    super::super::workflow_live_v2_host_dispatch::run_v2_agent_call_with_rejected_output_log(
        &adapter, &client, &request, None,
    )
    .await
    .expect("the refusal stopped the call instead of starting a new agent");
    let calls = port.calls.lock().unwrap();
    let kinds: Vec<_> = calls.iter().map(|call| call.0).collect();
    assert_eq!(kinds, vec!["fresh", "continue", "fresh"]);
    assert_ne!(
        calls[2].1, calls[0].1,
        "the new agent reused the old session"
    );
    assert!(
        calls[2].2.contains("invalid initial answer"),
        "no prior findings"
    );
}

/// An author re-ask whose session cannot be continued starts a new author.
#[tokio::test]
async fn a_refused_author_continuation_starts_a_new_author() {
    let port = RefusingContinuation::new(&[&accepted(), &accepted()]);
    let (client, _rx) = live(port.clone());
    let mut request = tests::request(WorkflowV2HostMethod::Agent, None);
    request.call.id = "author-workflow-script".into();
    let adapter = archon_workflow::WorkflowV2AgentAdapter::new();
    archon_workflow::v2::repair_session::author_scope(async {
        for _ in 0..2 {
            super::super::workflow_live_v2_host_dispatch::run_v2_agent_call_with_rejected_output_log(
                &adapter, &client, &request, None,
            )
            .await
            .expect("a refused author continuation stopped the run");
        }
    })
    .await;
    let kinds: Vec<_> = port.calls.lock().unwrap().iter().map(|c| c.0).collect();
    assert_eq!(kinds, vec!["fresh", "continue", "fresh"]);
}
