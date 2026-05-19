use std::future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use archon_tools::agent_tool::{SubagentRequest, run_subagent};
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor, SubagentOutcome,
    install_subagent_executor,
};
use archon_tools::tool::{AgentMode, ToolContext};
use tokio_util::sync::CancellationToken;

struct HangingExecutor {
    started: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
    inner_complete: Arc<AtomicBool>,
    visible_complete: Arc<AtomicBool>,
}

struct DropMarker {
    dropped: Arc<AtomicBool>,
}

impl Drop for DropMarker {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl SubagentExecutor for HangingExecutor {
    async fn run_to_completion(
        &self,
        _subagent_id: String,
        _request: SubagentRequest,
        _ctx: ToolContext,
        _cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        let _marker = DropMarker {
            dropped: Arc::clone(&self.dropped),
        };
        self.started.store(true, Ordering::SeqCst);
        future::pending::<()>().await;
        unreachable!("pending executor should only finish when aborted");
    }

    async fn on_inner_complete(&self, _subagent_id: String, _result: Result<String, String>) {
        self.inner_complete.store(true, Ordering::SeqCst);
    }

    async fn on_visible_complete(
        &self,
        _subagent_id: String,
        _result: Result<String, String>,
        _nested: bool,
    ) -> OutcomeSideEffects {
        self.visible_complete.store(true, Ordering::SeqCst);
        OutcomeSideEffects::default()
    }

    fn auto_background_ms(&self) -> u64 {
        0
    }

    fn classify(&self, _request: &SubagentRequest) -> SubagentClassification {
        SubagentClassification::Foreground
    }
}

fn make_ctx() -> ToolContext {
    ToolContext {
        working_dir: PathBuf::from("/tmp"),
        session_id: "subagent-cancel-abort-test".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

async fn wait_for(flag: &AtomicBool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if flag.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("flag did not flip before timeout");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_subagent_aborts_executor_task() {
    let executor = Arc::new(HangingExecutor {
        started: Arc::new(AtomicBool::new(false)),
        dropped: Arc::new(AtomicBool::new(false)),
        inner_complete: Arc::new(AtomicBool::new(false)),
        visible_complete: Arc::new(AtomicBool::new(false)),
    });
    install_subagent_executor(executor.clone());

    let cancel = CancellationToken::new();
    let handle = tokio::spawn(run_subagent(
        "cancel-abort".into(),
        SubagentRequest {
            prompt: "hang until cancelled".into(),
            model: None,
            allowed_tools: Vec::new(),
            max_turns: 10,
            timeout_secs: 300,
            subagent_type: None,
            run_in_background: false,
            cwd: None,
            isolation: None,
        },
        cancel.clone(),
        make_ctx(),
    ));

    wait_for(&executor.started).await;
    cancel.cancel();

    let outcome = tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("run_subagent should return promptly after cancellation")
        .expect("run_subagent task should not panic");
    assert!(matches!(outcome, SubagentOutcome::Cancelled));
    wait_for(&executor.dropped).await;
    wait_for(&executor.inner_complete).await;
    wait_for(&executor.visible_complete).await;
}
