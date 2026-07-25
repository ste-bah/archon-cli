use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use archon_core::agent::AgentConfig;
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::ToolRegistry;
use archon_core::subagent::{SubagentManager, SubagentStatus};
use archon_core::subagent_executor::AgentSubagentExecutor;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_tools::subagent_executor::install_subagent_executor;
use archon_tools::task_create::TaskCreateTool;
use archon_tools::task_manager::{TASK_MANAGER, TaskStatus};
use archon_tools::task_stop::TaskStopTool;
use archon_tools::tool::{Tool, ToolContext};
use tokio::sync::{mpsc, oneshot};

struct StalledProvider {
    started: Mutex<Option<oneshot::Sender<()>>>,
    dropped: Mutex<Option<oneshot::Sender<()>>>,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl LlmProvider for StalledProvider {
    fn name(&self) -> &str {
        "stalled"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(&self, _: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel(1);
        let dropped = self.dropped.lock().unwrap().take().unwrap();
        tokio::spawn(async move {
            tx.closed().await;
            let _ = dropped.send(());
        });
        self.started.lock().unwrap().take().unwrap().send(()).ok();
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("stalled provider only streams")
    }
}

fn install_real_executor(
    provider: Arc<StalledProvider>,
    root: &std::path::Path,
) -> Arc<tokio::sync::Mutex<SubagentManager>> {
    let manager = Arc::new(tokio::sync::Mutex::new(SubagentManager::new(1)));
    let executor = AgentSubagentExecutor::new(
        provider,
        ToolRegistry::new(),
        Arc::clone(&manager),
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(root))),
        None,
        None,
        root.to_path_buf(),
        "task-create-real-cancel".into(),
        "stalled-model".into(),
        Vec::new(),
        Arc::new(tokio::sync::Mutex::new("default".to_string())),
        Arc::new(tokio::sync::Mutex::new(None)),
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "task-create-real-cancel".into(),
            String::new(),
            String::new(),
        )),
    );
    install_subagent_executor(Arc::new(executor));
    manager
}

fn tool_context(root: &std::path::Path) -> ToolContext {
    ToolContext {
        working_dir: root.to_path_buf(),
        session_id: "task-create-real-cancel".into(),
        ..ToolContext::default()
    }
}

async fn wait_for_failed_agent(manager: &tokio::sync::Mutex<SubagentManager>, agent_id: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let failed = manager
                .lock()
                .await
                .get_status(agent_id)
                .is_some_and(|info| matches!(info.status, SubagentStatus::Failed(_)));
            if failed {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("real executor should reach terminal manager state");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn task_stop_cancels_real_executor_during_stalled_inference() {
    let root = tempfile::tempdir().expect("project root");
    let (started_tx, started_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let provider = Arc::new(StalledProvider {
        started: Mutex::new(Some(started_tx)),
        dropped: Mutex::new(Some(dropped_tx)),
        calls: AtomicUsize::new(0),
    });
    let manager = install_real_executor(Arc::clone(&provider), root.path());
    let ctx = tool_context(root.path());
    let created = TaskCreateTool
        .execute(
            serde_json::json!({
                "subject": "Cancel real inference",
                "description": "stalled provider",
                "prompt": "wait forever",
                "run_in_background": true
            }),
            &ctx,
        )
        .await;
    let response: serde_json::Value = serde_json::from_str(&created.content).expect("create JSON");
    let task_id = response["task_id"].as_str().expect("task id");
    let agent_id = response["agent_id"].as_str().expect("agent id");
    assert_eq!(response["status"], "spawned");
    tokio::time::timeout(std::time::Duration::from_secs(1), started_rx)
        .await
        .expect("provider should start")
        .expect("start signal");

    let stopped = TaskStopTool
        .execute(serde_json::json!({"task_id": task_id}), &ctx)
        .await;

    assert!(!stopped.is_error);
    tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
        .await
        .expect("provider receiver should drop")
        .expect("drop signal");
    wait_for_failed_agent(&manager, agent_id).await;
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        TASK_MANAGER.get_task(task_id).expect("tracked task").status,
        TaskStatus::Stopped
    );
}
