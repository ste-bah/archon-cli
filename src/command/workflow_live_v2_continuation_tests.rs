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

/// The executor refuses to continue the first session (it cannot restore it
/// exactly, #241) and continues any later one. Every call answers from
/// `replies` in order. `typed: false` raises the same words as an ordinary
/// error, as a model value quoted into a validation error would.
struct RefusingContinuation {
    replies: Mutex<Vec<String>>,
    calls: Mutex<Vec<(&'static str, String, String)>>,
    typed: bool,
}
impl RefusingContinuation {
    fn new(replies: &[&str]) -> Arc<Self> {
        Self::build(replies, true)
    }
    fn build(replies: &[&str], typed: bool) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.iter().rev().map(|r| r.to_string()).collect()),
            calls: Mutex::new(vec![]),
            typed,
        })
    }
    fn record(&self, kind: &'static str, call: &WorkflowAgentCall) -> String {
        let prompt = serde_json::to_string(&call.messages).unwrap();
        let mut calls = self.calls.lock().unwrap();
        calls.push((kind, call.session_id.clone(), prompt));
        calls[0].1.clone()
    }
    fn reply(&self) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        Ok(outcome(
            self.replies.lock().unwrap().pop().expect("a reply"),
        ))
    }
    fn kinds(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|call| call.0)
            .collect()
    }
    fn session(&self, index: usize) -> String {
        self.calls.lock().unwrap()[index].1.clone()
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
        self.reply()
    }
    async fn continue_agent(
        &self,
        call: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        if self.record("continue", &call) != call.session_id {
            return self.reply();
        }
        let words = format!(
            "cannot continue agent '{}': its confinement is only known to the process that \
             started it; start a new agent",
            call.session_id
        );
        Err(match self.typed {
            true => archon_workflow::WorkflowError::port(
                archon_tools::subagent_session::ContinuationRefused(words),
            ),
            false => archon_workflow::WorkflowError::port(anyhow::anyhow!("{words}")),
        })
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

/// Valid JSON that fails the result contract: a different error class from
/// a malformed reply, so it earns the second repair.
fn accepted_without_evidence() -> String {
    serde_json::to_string(&archon_workflow::WorkflowV2Result::accepted("no evidence")).unwrap()
}

/// The client, and the receiver its activity sink needs kept open.
fn live(port: Arc<RefusingContinuation>) -> (LiveV2AgentClient, impl Sized) {
    let (sink, rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(port, sink, vec![], "refused-run".into(), None, Some(10));
    (client, rx)
}

async fn call(
    client: &LiveV2AgentClient,
    request: &archon_workflow::WorkflowV2AgentRequest,
) -> Result<archon_workflow::WorkflowV2Result, archon_workflow::WorkflowV2AgentError> {
    let adapter = archon_workflow::WorkflowV2AgentAdapter::new();
    super::super::workflow_live_v2_host_dispatch::run_v2_agent_call_with_rejected_output_log(
        &adapter, client, request, None,
    )
    .await
}

/// A refused repair continuation starts an explicit new agent, in a new
/// session, told the prior attempt's findings. When that agent's answer needs
/// the second repair, the repair continues the new agent, not the refused one.
#[tokio::test]
async fn a_refused_repair_starts_a_new_agent_whose_session_later_repairs_continue() {
    let port = RefusingContinuation::new(&[
        "invalid initial answer",
        &accepted_without_evidence(),
        &accepted(),
    ]);
    let (client, _rx) = live(port.clone());
    let request = tests::request(WorkflowV2HostMethod::Agent, None);
    call(&client, &request)
        .await
        .expect("the refusal stopped the call instead of starting a new agent");
    assert_eq!(port.kinds(), vec!["fresh", "continue", "fresh", "continue"]);
    assert_ne!(
        port.session(2),
        port.session(0),
        "the new agent reused the old session"
    );
    assert_eq!(
        port.session(3),
        port.session(2),
        "the second repair went to the refused session"
    );
    assert!(
        port.calls.lock().unwrap()[2]
            .2
            .contains("invalid initial answer")
    );
}

/// An author re-ask whose session cannot be continued starts a new author,
/// and every later re-ask continues that new author.
#[tokio::test]
async fn a_refused_author_continuation_starts_a_new_author_that_later_calls_continue() {
    let port = RefusingContinuation::new(&[&accepted(), &accepted(), &accepted()]);
    let (client, _rx) = live(port.clone());
    let mut request = tests::request(WorkflowV2HostMethod::Agent, None);
    request.call.id = "author-workflow-script".into();
    archon_workflow::v2::repair_session::author_scope(async {
        for _ in 0..3 {
            call(&client, &request)
                .await
                .expect("a refused author continuation stopped the run");
        }
    })
    .await;
    assert_eq!(port.kinds(), vec!["fresh", "continue", "fresh", "continue"]);
    assert_eq!(
        port.session(3),
        port.session(2),
        "the third call went to the refused author"
    );
}

/// The refusal's words in an ordinary error (a model value quoted into a
/// validation error) are not a refusal: no new agent is started.
#[tokio::test]
async fn refusal_words_in_an_ordinary_error_do_not_start_a_new_agent() {
    let port = RefusingContinuation::build(&["invalid initial answer"], false);
    let (client, _rx) = live(port.clone());
    let request = tests::request(WorkflowV2HostMethod::Agent, None);
    call(&client, &request)
        .await
        .expect_err("an ordinary error was taken for a refusal");
    assert_eq!(port.kinds(), vec!["fresh", "continue"]);
}
