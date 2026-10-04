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

/// A client that keeps no session cannot continue one. The repair is not
/// disguised as a continuation: an explicit new agent is started, told the
/// prior attempt's findings, and the call proceeds (#241).
#[tokio::test]
async fn a_client_without_sessions_gets_an_explicit_new_agent_for_the_repair() {
    struct Stateless(Mutex<Vec<String>>);
    #[async_trait::async_trait]
    impl WorkflowV2AgentClient for Stateless {
        async fn run_agent(&self, prompt: String) -> Result<String, WorkflowV2AgentError> {
            let mut prompts = self.0.lock().unwrap();
            prompts.push(prompt);
            if prompts.len() == 1 {
                return Ok("invalid initial answer".into());
            }
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
    let client = Stateless(Mutex::new(Vec::new()));
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
        .expect("the refused continuation stopped the call");
    let prompts = client.0.lock().unwrap();
    assert_eq!(prompts.len(), 2, "no explicit new agent was started");
    assert!(
        prompts[1].contains("inspect source"),
        "the new agent lost the task"
    );
    assert!(
        prompts[1].contains("invalid initial answer"),
        "the new agent was not told the prior findings"
    );
}
