use super::super::*;
use super::*;

struct SkeletonPort;
#[async_trait::async_trait]
impl WorkflowLlmClient for SkeletonPort {
    async fn continue_agent(
        &self,
        call: WorkflowAgentCall,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.run_agent(call).await
    }
    async fn send_message(
        &self,
        _: Vec<Value>,
        _: Vec<Value>,
        _: Vec<Value>,
        _: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!()
    }
    async fn run_agent(&self, _: WorkflowAgentCall) -> WorkflowResult<WorkflowAgentOutcome> {
        let tool = archon_tools::audit_landing::current().expect("agent started with landing tool");
        assert_eq!(tool.tool_name(), "land-skeleton-record");
        tool.land(json!({"subject":"TASK-X-001","evidence":[{"kind":"inspection","summary":"Mapped the task to the listed requirement"}],"task":{"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["REQ-X-001"]}}))
            .map_err(WorkflowError::SpecInvalid)?;
        Ok(WorkflowAgentOutcome {
            content: json!({"status":"accepted","data":{"records_landed":1}}).to_string(),
            tool_uses: vec![],
            tokens_in: Some(0),
            tokens_out: Some(0),
            stop_reason: Some("end_turn".into()),
        })
    }
}
#[tokio::test]
#[allow(non_snake_case)]
async fn R1_P5_28_host_bad_seed_next_call_and_resume_dispatch_agent() {
    let temp = tempfile::tempdir().unwrap();
    let (sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(SkeletonPort),
        sink,
        vec![],
        "fixture".into(),
        None,
        Some(30),
    );
    let valid =
        json!({"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["REQ-X-001"]});
    let mut seeds = vec![json!("Landed tasks."), Value::Null];
    for (field, value) in [
        ("task_id", json!("invalid")),
        ("file_name", json!("../invalid.md")),
        ("implements", json!([])),
        ("implements", json!([" "])),
    ] {
        let mut task = valid.clone();
        task[field] = value;
        seeds.push(json!({"schema_version":1,"acceptance_digest":"","tasks":[task.clone()]}));
        seeds.push(json!({"schema_version":1,"acceptance_digest":"","tasks":[task,valid.clone()]}));
    }
    // Each request gets a new store, including a resumed call carrying pre-upgrade bytes.
    for seed in seeds
        .into_iter()
        .flat_map(|seed| [seed.clone(), json!(seed.to_string())])
    {
        for frozen in [false, true] {
            let store =
                WorkflowV2ResultStore::new(temp.path().join(uuid::Uuid::new_v4().to_string()));
            let execution = WorkflowV2CallExecution {
                call: WorkflowV2HostCall {
                    id: "skeleton".into(),
                    method: WorkflowV2HostMethod::Agent,
                    write_mode: None,
                    options: WorkflowV2HostOptions {
                        extra: [
                            ("recordLanding".into(), json!("skeleton")),
                            ("previousSkeleton".into(), seed.clone()),
                            ("frozenSkeleton".into(), json!(frozen)),
                        ]
                        .into(),
                        ..Default::default()
                    },
                },
                input: json!({}),
                depends_on: vec![],
            };
            let request = archon_workflow::v2::call_data::v2_agent_request(
                "skeleton", None, &execution, None,
            );
            let result = run_v2_agent_call_with_rejected_output_log(
                &WorkflowV2AgentAdapter::new(),
                &client,
                &request,
                Some(&store),
            )
            .await
            .unwrap();
            assert_eq!(result.data["records_landed"], 1);
            assert_eq!(
                result.evidence[0].summary,
                "Mapped the task to the listed requirement"
            );
        }
    }
}
