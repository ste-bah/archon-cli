use std::sync::{Arc, RwLock};

use archon_tools::gametheory::{
    GameTheoryCallSpecialistRequest, GameTheoryClassifyRequest, GameTheoryExecutor,
    GameTheoryInspectRequest, GameTheoryListAgentsRequest, GameTheoryReplayRequest,
    GameTheoryRunRequest, GameTheorySpecimensRequest, GameTheoryStatus, GameTheoryStatusRequest,
    install_gametheory_executor,
};
use archon_tools::tool::{Tool, ToolContext};
use async_trait::async_trait;
use serde_json::json;
use serial_test::serial;

#[derive(Clone, Default)]
struct RecordingExecutor {
    calls: Arc<RwLock<Vec<String>>>,
}

#[async_trait]
impl GameTheoryExecutor for RecordingExecutor {
    async fn run(&self, request: GameTheoryRunRequest) -> anyhow::Result<String> {
        self.record(format!("run:{}", request.situation));
        Ok("run-ok".into())
    }

    async fn status(&self, request: GameTheoryStatusRequest) -> anyhow::Result<String> {
        self.record(format!(
            "status:{}",
            request.run_id.unwrap_or_else(|| "<latest>".into())
        ));
        Ok("status-ok".into())
    }

    async fn list_agents(&self, request: GameTheoryListAgentsRequest) -> anyhow::Result<String> {
        self.record(format!("list-agents:{:?}", request.tier));
        Ok("list-agents-ok".into())
    }

    async fn specimens(&self, request: GameTheorySpecimensRequest) -> anyhow::Result<String> {
        self.record(format!("specimens:{:?}:{}", request.filter, request.ingest));
        Ok("specimens-ok".into())
    }

    async fn inspect(&self, request: GameTheoryInspectRequest) -> anyhow::Result<String> {
        self.record(format!("inspect:{}", request.artifact_id));
        Ok("inspect-ok".into())
    }

    async fn replay(&self, request: GameTheoryReplayRequest) -> anyhow::Result<String> {
        self.record(format!("replay:{}", request.run_id));
        Ok("replay-ok".into())
    }

    async fn classify(&self, request: GameTheoryClassifyRequest) -> anyhow::Result<String> {
        self.record(format!("classify:{}", request.situation));
        Ok("classify-ok".into())
    }

    async fn call_specialist(
        &self,
        request: GameTheoryCallSpecialistRequest,
    ) -> anyhow::Result<String> {
        self.record(format!(
            "call-specialist:{}:{}",
            request.run_id, request.agent_key
        ));
        Ok("call-specialist-ok".into())
    }
}

impl RecordingExecutor {
    fn record(&self, entry: String) {
        self.calls.write().unwrap().push(entry);
    }
}

#[tokio::test]
#[serial]
async fn test_gametheory_status_tool_invokes_installed_executor_source_of_truth() {
    let recorder = RecordingExecutor::default();
    let calls = recorder.calls.clone();
    install_gametheory_executor(Arc::new(recorder));

    let result = GameTheoryStatus
        .execute(
            json!({ "run_id": "gt-source-truth" }),
            &ToolContext::default(),
        )
        .await;

    assert!(!result.is_error);
    assert_eq!(result.content, "status-ok");
    assert_eq!(
        calls.read().unwrap().as_slice(),
        &["status:gt-source-truth"],
        "the source of truth is the executor call log, read after tool execution"
    );
}
