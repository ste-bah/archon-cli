use std::sync::Mutex;

use archon_workflow::{
    WorkflowV2AgentAdapter, WorkflowV2AgentClient, WorkflowV2AgentError, WorkflowV2AgentRequest,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2Result,
};

struct ContinuationClient {
    calls: Mutex<Vec<&'static str>>,
}

#[async_trait::async_trait]
impl WorkflowV2AgentClient for ContinuationClient {
    async fn run_agent(&self, _: String) -> Result<String, WorkflowV2AgentError> {
        self.calls.lock().unwrap().push("fresh");
        Ok("invalid initial answer".into())
    }

    async fn continue_agent_request(
        &self,
        request: &WorkflowV2AgentRequest,
        prompt: String,
    ) -> Result<String, WorkflowV2AgentError> {
        assert_eq!(request.call.id, "same-call");
        assert!(prompt.contains("invalid initial answer"));
        self.calls.lock().unwrap().push("continue");
        let mut result = WorkflowV2Result::accepted("inspected source");
        result
            .evidence
            .push(archon_workflow::WorkflowV2Evidence::new(
                archon_workflow::WorkflowV2EvidenceKind::Inspection,
                "inspected source contents",
            ));
        Ok(serde_json::to_string(&result).unwrap())
    }
}

#[tokio::test]
async fn validation_repair_uses_explicit_continuation_not_fresh_dispatch() {
    let client = ContinuationClient {
        calls: Mutex::new(Vec::new()),
    };
    let request = WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "same-call".into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        role: "researcher".into(),
        task: "inspect source".into(),
        constraints: vec![],
        input: serde_json::Value::Null,
        repository_root: None,
        project_artifacts: Default::default(),
        target_files: vec![],
        target_ownership_scopes: vec![],
    };
    WorkflowV2AgentAdapter::new()
        .run_with_repair(&client, &request)
        .await
        .unwrap();
    assert_eq!(*client.calls.lock().unwrap(), vec!["fresh", "continue"]);
}
