use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use archon_tools::agent_tool::SubagentRequest;
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
    install_subagent_executor,
};
use archon_tools::task_create::TaskCreateTool;
use archon_tools::task_manager::{TASK_MANAGER, TaskStatus};
use archon_tools::tool::{Tool, ToolContext};
use async_trait::async_trait;
use serde_json::json;
use tokio_util::sync::CancellationToken;

struct CancellationExecutor {
    auto_bg_ms: AtomicU64,
    delay_ms: AtomicU64,
    wait_for_cancel: AtomicBool,
    ran: AtomicBool,
    cancel_observed: AtomicBool,
    inner_completed: AtomicBool,
    inner_completion_count: AtomicUsize,
    visible_completed: AtomicBool,
    cancel_before_return: Mutex<Option<CancellationToken>>,
}

#[async_trait]
impl SubagentExecutor for CancellationExecutor {
    async fn run_to_completion(
        &self,
        subagent_id: String,
        _request: SubagentRequest,
        _ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        self.ran.store(true, Ordering::SeqCst);
        if self.wait_for_cancel.load(Ordering::SeqCst) {
            cancel.cancelled().await;
            self.cancel_observed.store(true, Ordering::SeqCst);
            self.on_inner_complete(subagent_id, Err("cancelled".into()))
                .await;
            return Err(ExecutorError::Internal("cancelled".into()));
        }
        tokio::time::sleep(std::time::Duration::from_millis(
            self.delay_ms.load(Ordering::SeqCst),
        ))
        .await;
        self.on_inner_complete(subagent_id, Ok("recorded".into()))
            .await;
        if let Some(parent) = self
            .cancel_before_return
            .lock()
            .expect("cancel-before-return token")
            .take()
        {
            parent.cancel();
        }
        Ok("recorded".into())
    }

    async fn on_inner_complete(&self, _subagent_id: String, _result: Result<String, String>) {
        self.inner_completed.store(true, Ordering::SeqCst);
        self.inner_completion_count.fetch_add(1, Ordering::SeqCst);
    }

    async fn on_visible_complete(
        &self,
        _subagent_id: String,
        _result: Result<String, String>,
        _nested: bool,
    ) -> OutcomeSideEffects {
        self.visible_completed.store(true, Ordering::SeqCst);
        OutcomeSideEffects::default()
    }

    fn auto_background_ms(&self) -> u64 {
        self.auto_bg_ms.load(Ordering::SeqCst)
    }

    fn classify(&self, request: &SubagentRequest) -> SubagentClassification {
        if request.run_in_background {
            SubagentClassification::ExplicitBackground
        } else {
            SubagentClassification::Foreground
        }
    }
}

fn executor() -> Arc<CancellationExecutor> {
    static EXECUTOR: OnceLock<Arc<CancellationExecutor>> = OnceLock::new();
    let executor = EXECUTOR
        .get_or_init(|| {
            Arc::new(CancellationExecutor {
                auto_bg_ms: AtomicU64::new(0),
                delay_ms: AtomicU64::new(0),
                wait_for_cancel: AtomicBool::new(false),
                ran: AtomicBool::new(false),
                cancel_observed: AtomicBool::new(false),
                inner_completed: AtomicBool::new(false),
                inner_completion_count: AtomicUsize::new(0),
                visible_completed: AtomicBool::new(false),
                cancel_before_return: Mutex::new(None),
            })
        })
        .clone();
    install_subagent_executor(executor.clone());
    executor
}

fn context(parent: Option<CancellationToken>) -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "task-create-cancellation-test".into(),
        cancel_parent: parent,
        ..Default::default()
    }
}

fn prompted_task(description: &str, background: bool) -> serde_json::Value {
    json!({
        "subject": "Cancellation",
        "description": description,
        "prompt": "Run until the test completes",
        "run_in_background": background
    })
}

fn task_id(result: &archon_tools::tool::ToolResult) -> String {
    serde_json::from_str::<serde_json::Value>(&result.content).expect("response json")["task_id"]
        .as_str()
        .expect("task id")
        .to_string()
}

async fn wait_for_status(task_id: &str, expected: TaskStatus) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while TASK_MANAGER
            .get_task(task_id)
            .is_none_or(|task| task.status != expected)
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("task {task_id} must reach {expected}"));
}

async fn wait_for(flag: &AtomicBool, message: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !flag.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{message}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn task_stop_cancels_explicit_background_executor() {
    let executor = executor();
    executor.wait_for_cancel.store(true, Ordering::SeqCst);
    executor.cancel_observed.store(false, Ordering::SeqCst);
    executor.inner_completion_count.store(0, Ordering::SeqCst);
    executor.visible_completed.store(false, Ordering::SeqCst);
    let result = TaskCreateTool
        .execute(prompted_task("explicit background", true), &context(None))
        .await;
    let task_id = task_id(&result);

    TASK_MANAGER.stop_task(&task_id).expect("stop task");
    wait_for(&executor.cancel_observed, "executor must observe TaskStop").await;
    wait_for_status(&task_id, TaskStatus::Stopped).await;
    wait_for(
        &executor.visible_completed,
        "visible completion must finish",
    )
    .await;
    assert_eq!(executor.inner_completion_count.load(Ordering::SeqCst), 1);
    executor.wait_for_cancel.store(false, Ordering::SeqCst);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn parent_cancellation_stops_auto_background_executor() {
    let executor = executor();
    executor.auto_bg_ms.store(1, Ordering::SeqCst);
    executor.wait_for_cancel.store(true, Ordering::SeqCst);
    executor.cancel_observed.store(false, Ordering::SeqCst);
    let parent = CancellationToken::new();
    let result = TaskCreateTool
        .execute(
            prompted_task("auto background", false),
            &context(Some(parent.clone())),
        )
        .await;
    let task_id = task_id(&result);

    parent.cancel();
    wait_for(
        &executor.cancel_observed,
        "executor must observe parent cancel",
    )
    .await;
    wait_for_status(&task_id, TaskStatus::Stopped).await;
    executor.auto_bg_ms.store(0, Ordering::SeqCst);
    executor.wait_for_cancel.store(false, Ordering::SeqCst);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn parent_cancellation_stops_foreground_executor() {
    let executor = executor();
    executor.ran.store(false, Ordering::SeqCst);
    executor.wait_for_cancel.store(true, Ordering::SeqCst);
    executor.cancel_observed.store(false, Ordering::SeqCst);
    let parent = CancellationToken::new();
    let execution = tokio::spawn({
        let parent = parent.clone();
        async move {
            TaskCreateTool
                .execute(prompted_task("foreground", false), &context(Some(parent)))
                .await
        }
    });
    wait_for(&executor.ran, "foreground executor must start").await;
    let task_id = TASK_MANAGER
        .list_tasks()
        .into_iter()
        .find(|task| task.description == "Cancellation: foreground")
        .expect("foreground task")
        .id;

    parent.cancel();
    wait_for(
        &executor.cancel_observed,
        "executor must observe parent cancel",
    )
    .await;
    assert!(execution.await.expect("TaskCreate execution").is_error);
    wait_for_status(&task_id, TaskStatus::Stopped).await;
    executor.wait_for_cancel.store(false, Ordering::SeqCst);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn auto_background_completion_wins_same_poll_parent_cancel() {
    let executor = executor();
    executor.auto_bg_ms.store(1, Ordering::SeqCst);
    executor.delay_ms.store(10, Ordering::SeqCst);
    let parent = CancellationToken::new();
    *executor
        .cancel_before_return
        .lock()
        .expect("cancel-before-return token") = Some(parent.clone());
    let result = TaskCreateTool
        .execute(
            prompted_task("same-poll race", false),
            &context(Some(parent)),
        )
        .await;
    let task_id = task_id(&result);

    wait_for_status(&task_id, TaskStatus::Completed).await;
    executor.auto_bg_ms.store(0, Ordering::SeqCst);
    executor.delay_ms.store(0, Ordering::SeqCst);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn task_completion_wins_before_late_stop() {
    let executor = executor();
    executor.delay_ms.store(0, Ordering::SeqCst);
    executor.wait_for_cancel.store(false, Ordering::SeqCst);
    let result = TaskCreateTool
        .execute(prompted_task("late stop", true), &context(None))
        .await;
    let task_id = task_id(&result);
    wait_for_status(&task_id, TaskStatus::Completed).await;

    TASK_MANAGER.stop_task(&task_id).expect("late stop call");
    assert_eq!(
        TASK_MANAGER.get_task(&task_id).expect("task").status,
        TaskStatus::Completed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn auto_background_returns_without_visible_completion() {
    let executor = executor();
    executor.auto_bg_ms.store(1, Ordering::SeqCst);
    executor.delay_ms.store(200, Ordering::SeqCst);
    executor.inner_completed.store(false, Ordering::SeqCst);
    executor.visible_completed.store(false, Ordering::SeqCst);
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        TaskCreateTool.execute(prompted_task("preserve D5", false), &context(None)),
    )
    .await
    .expect("timer must return before executor completion");
    let task_id = task_id(&result);

    wait_for_status(&task_id, TaskStatus::Completed).await;
    assert!(executor.inner_completed.load(Ordering::SeqCst));
    assert!(!executor.visible_completed.load(Ordering::SeqCst));
    executor.auto_bg_ms.store(0, Ordering::SeqCst);
    executor.delay_ms.store(0, Ordering::SeqCst);
}
