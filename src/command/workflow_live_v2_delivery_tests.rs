use super::*;
use archon_workflow::{WorkflowAgentOutcome, WorkflowV2HostCall, WorkflowV2HostMethod};

#[derive(Default)]
struct CompletionBlockedClient {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for CompletionBlockedClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("test uses run_agent")
    }

    async fn run_agent(
        &self,
        _request: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(WorkflowAgentOutcome {
            content: "recorded".to_string(),
            tool_uses: Vec::new(),
            tokens_in: 0,
            tokens_out: 0,
        })
    }
}

fn request() -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "discover".to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        role: "researcher".to_string(),
        task: "inspect repository and task files".to_string(),
        constraints: Vec::new(),
        input: serde_json::json!({ "objective": "test" }),
        repository_root: Some("/repo".to_string()),
        project_artifacts: Default::default(),
        target_files: vec!["src/lib.rs".to_string()],
        target_ownership_scopes: Vec::new(),
    }
}

#[tokio::test]
async fn closed_tui_prevents_v2_success_publication() {
    let recorder = Arc::new(CompletionBlockedClient::default());
    let (tui_tx, mut tui_rx) = archon_tui::event_channel::bounded_tui_event_channel();
    let client = LiveV2AgentClient::new(
        recorder.clone(),
        tui_tx,
        Vec::new(),
        "wf-test".to_string(),
        Some("/repo".to_string()),
        Some(17),
    );
    let handle = tokio::spawn(async move {
        client
            .run_agent_request(&request(), "inspect".to_string())
            .await
    });

    recorder.started.notified().await;
    let _running = tui_rx.recv().await.expect("running activity");
    drop(tui_rx);
    recorder.release.notify_one();

    handle
        .await
        .expect("V2 join")
        .expect_err("closed TUI must prevent V2 success publication");
}
