use super::*;

#[tokio::test]
async fn host_dispatch_runner_landing_failure_becomes_invalid_answer() {
    struct FailingRunner(bool);
    #[async_trait::async_trait]
    impl archon_workflow::WorkflowLlmClient for FailingRunner {
        async fn send_message(
            &self,
            _: Vec<Value>,
            _: Vec<Value>,
            _: Vec<Value>,
            _: &str,
        ) -> WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
            unreachable!()
        }
        async fn run_agent(
            &self,
            _: archon_workflow::WorkflowAgentCall,
        ) -> WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
            if !self.0 {
                return Err(WorkflowError::StageFailed(
                    "invalid_answer: transport text".into(),
                ));
            }
            let records = archon_workflow::v2::record_landing::current().unwrap();
            let rounds = (0..3).map(|_| (vec![], reply("Done."))).collect();
            let (runner, _) = runner(records, rounds, false);
            let error = runner.run("Repair tasks").await.unwrap_err();
            assert!(
                error
                    .downcast_ref::<archon_tools::audit_landing::IncompleteLandedArtifact>()
                    .is_some()
            );
            Err(WorkflowError::port(
                error.reallocate_into_boxed_dyn_error_without_backtrace(),
            ))
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let (sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    for typed in [true, false] {
        let client = super::super::super::LiveV2AgentClient::new(
            Arc::new(FailingRunner(typed)),
            sink.clone(),
            vec![],
            "run".into(),
            None,
            Some(30),
        )
        .with_fixed_raw_tool_policy(vec!["Read".into()]);
        let execution = WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: "skeleton-author-1".into(),
                method: WorkflowV2HostMethod::Agent,
                write_mode: None,
                options: WorkflowV2HostOptions {
                    result_mode: Some(archon_workflow::AgentResultMode::RawOutcome),
                    extra: [
                        ("recordLanding".into(), json!("skeleton")),
                        ("openFindings".into(), json!(["repair task"])),
                    ]
                    .into(),
                    ..Default::default()
                },
            },
            input: json!({}),
            depends_on: vec![],
        };
        let result = super::super::super::run_single_v2_agent_call(
            "Author tasks",
            None,
            &execution,
            &WorkflowV2AgentAdapter::new(),
            &client,
            Some(&store),
            None,
            true,
        )
        .await;
        if typed {
            let result = result.unwrap();
            assert_eq!(result.data["invalid_answer"], true);
            assert!(!result.data.to_string().contains("host_fault"));
        } else {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("invalid_answer: transport text")
            );
        }
    }
}
