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
        Ok(outcome(
            serde_json::to_string(&archon_workflow::WorkflowV2Result::accepted("inspected"))
                .unwrap(),
        ))
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
