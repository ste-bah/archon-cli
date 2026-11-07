use super::*;
use serde_json::{Value, json};
use std::sync::Mutex;

struct StoppedPort {
    transport: bool,
    records: Mutex<Option<Arc<archon_workflow::v2::record_landing::RecordLanding>>>,
}
#[async_trait::async_trait]
impl WorkflowLlmClient for StoppedPort {
    async fn send_message(
        &self,
        _: Vec<Value>,
        _: Vec<Value>,
        _: Vec<Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        unreachable!()
    }
    async fn run_agent(
        &self,
        _: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        let records = archon_workflow::v2::record_landing::current().expect("landing scope");
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/landing_reply_corpus/landings.json"
        ))
        .unwrap();
        records.land(fixture[0]["input"].clone())?;
        let receipt = records.retained_reply()?.expect("new record can complete");
        *self.records.lock().unwrap() = Some(records);
        if self.transport {
            // The resend window has already exhausted at the host boundary;
            // its typed cutoff must not start another provider retry loop.
            return Err(WorkflowError::HostCallTimeout(format!(
                "{} stream idle timeout",
                archon_llm::transport_idle::TRANSPORT_STALL_MARKER
            )));
        }
        Ok(archon_workflow::WorkflowAgentOutcome {
            content: receipt.to_string(),
            stop_reason: Some("shutdown".into()),
            ..Default::default()
        })
    }
}

#[tokio::test]
async fn host_dispatch_stops_never_promote_partial_landings_to_success() {
    for transport in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let port = Arc::new(StoppedPort {
            transport,
            records: Mutex::new(None),
        });
        let (sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
        let client = LiveV2AgentClient::new(
            port.clone(),
            sink,
            vec![],
            "stop-fixture".into(),
            None,
            Some(30),
        )
        .with_fixed_raw_tool_policy(vec!["Read".into()]);
        let execution = WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: "stop-author".into(),
                method: WorkflowV2HostMethod::Agent,
                write_mode: None,
                options: archon_workflow::WorkflowV2HostOptions {
                    result_mode: Some(archon_workflow::AgentResultMode::RawOutcome),
                    extra: [
                        ("recordLanding".into(), json!("skeleton")),
                        ("openFindings".into(), json!(["repair task contracts"])),
                    ]
                    .into(),
                    ..Default::default()
                },
            },
            input: json!({}),
            depends_on: vec![],
        };
        let result = run_single_v2_agent_call(
            "Author",
            None,
            &execution,
            &WorkflowV2AgentAdapter::new(),
            &client,
            Some(&store),
            None,
            true,
        )
        .await;
        if transport {
            let error = result.unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
                "{error}"
            );
        } else {
            let error = result.unwrap_err();
            assert!(
                matches!(&error, WorkflowError::ControlPaused(message) if message.contains("shutdown")),
                "{error}"
            );
        }
        let records = port.records.lock().unwrap().clone().unwrap();
        assert_eq!(records.new_landing_count().unwrap(), 1);
        assert!(records.retained_artifact().unwrap().is_some());
    }
}
