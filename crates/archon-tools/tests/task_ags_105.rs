//! TASK-AGS-105: SubagentExecutor trait contract tests.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::Notify;

use serde_json::json;

use archon_tools::agent_tool::{AgentTool, SubagentRequest};
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
    get_subagent_executor, install_subagent_executor,
};
use archon_tools::task_create::TaskCreateTool;
use archon_tools::tool::{Tool, ToolContext};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

// Recording executor shared by this serial test binary.

struct RecordingExecutor {
    ran: AtomicBool,
    visible_completed: AtomicBool,
    inner_completed: AtomicBool,
    auto_bg_ms: AtomicU64,
    run_delay_ms: AtomicU64,
    fail: AtomicBool,
    panic: AtomicBool,
    last_request: Mutex<Option<SubagentRequest>>,
    last_classified_request: Mutex<Option<SubagentRequest>>,
    last_nested: AtomicBool,
    run_count: AtomicUsize,
    run_notify: Notify,
}

#[async_trait]
impl SubagentExecutor for RecordingExecutor {
    async fn run_to_completion(
        &self,
        _subagent_id: String,
        request: SubagentRequest,
        ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        self.ran.store(true, Ordering::SeqCst);
        *self.last_request.lock().expect("record request") = Some(request);
        self.last_nested.store(ctx.nested, Ordering::SeqCst);
        self.run_count.fetch_add(1, Ordering::SeqCst);
        self.run_notify.notify_waiters();
        let fail = self.fail.load(Ordering::SeqCst);
        let panic = self.panic.load(Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(
            self.run_delay_ms.load(Ordering::SeqCst),
        ))
        .await;
        assert!(!panic, "recorded panic");
        let result = if fail {
            Err("recorded failure".to_string())
        } else {
            Ok(String::new())
        };
        self.on_inner_complete(String::new(), result.clone()).await;
        tokio::select! {
            _ = cancel.cancelled() => Err(ExecutorError::Internal("cancelled".into())),
            _ = std::future::ready(()) => match result {
                Ok(_) => Ok("recorded".into()),
                Err(error) => Err(ExecutorError::Internal(error)),
            },
        }
    }

    async fn on_inner_complete(&self, _subagent_id: String, _result: Result<String, String>) {
        self.inner_completed.store(true, Ordering::SeqCst);
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

    fn classify(&self, req: &SubagentRequest) -> SubagentClassification {
        *self
            .last_classified_request
            .lock()
            .expect("record classified request") = Some(req.clone());
        if req.run_in_background {
            SubagentClassification::ExplicitBackground
        } else {
            SubagentClassification::Foreground
        }
    }
}

fn recording_executor() -> Arc<RecordingExecutor> {
    static EXECUTOR: OnceLock<Arc<RecordingExecutor>> = OnceLock::new();
    let exec = EXECUTOR
        .get_or_init(|| {
            Arc::new(RecordingExecutor {
                ran: AtomicBool::new(false),
                visible_completed: AtomicBool::new(false),
                inner_completed: AtomicBool::new(false),
                auto_bg_ms: AtomicU64::new(0),
                run_delay_ms: AtomicU64::new(0),
                fail: AtomicBool::new(false),
                panic: AtomicBool::new(false),
                last_request: Mutex::new(None),
                last_classified_request: Mutex::new(None),
                last_nested: AtomicBool::new(false),
                run_count: AtomicUsize::new(0),
                run_notify: Notify::new(),
            })
        })
        .clone();
    install_subagent_executor(exec.clone());
    exec
}

fn make_ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "task-ags-105-test".into(),
        ..Default::default()
    }
}

async fn wait_for_task_status(task_id: &str, expected: archon_tools::task_manager::TaskStatus) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if archon_tools::task_manager::TASK_MANAGER
                .get_task(task_id)
                .is_some_and(|task| task.status == expected)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("TaskCreate task {task_id} must reach {expected}"));
}

// Test 1: trait remains object-safe. Historical test name retained so the
// baseline records the original contract identity.
// ---------------------------------------------------------------------------
#[test]
fn trait_is_object_safe_with_five_methods() {
    // Compile-time check: the trait can be used as `dyn SubagentExecutor`.
    fn _requires_object_safe(_x: Arc<dyn SubagentExecutor>) {}
    // Semantic check: a boxed trait object satisfies Send+Sync+'static so
    // it can live inside the global OnceLock.
    let e: Arc<dyn SubagentExecutor> = Arc::new(RecordingExecutor {
        ran: AtomicBool::new(false),
        visible_completed: AtomicBool::new(false),
        inner_completed: AtomicBool::new(false),
        auto_bg_ms: AtomicU64::new(0),
        run_delay_ms: AtomicU64::new(0),
        fail: AtomicBool::new(false),
        panic: AtomicBool::new(false),
        last_request: Mutex::new(None),
        last_classified_request: Mutex::new(None),
        last_nested: AtomicBool::new(false),
        run_count: AtomicUsize::new(0),
        run_notify: Notify::new(),
    });
    _requires_object_safe(e);
}

// Test 2: install_subagent_executor + get_subagent_executor round-trip.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn install_then_get_round_trips() {
    recording_executor();
    let exec = get_subagent_executor();
    assert!(
        exec.is_some(),
        "executor must be retrievable after install_subagent_executor"
    );
}

// ---------------------------------------------------------------------------
// Test 3: classify routes run_in_background:true to ExplicitBackground.
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn classify_routes_background_flag() {
    recording_executor();
    let exec = get_subagent_executor().expect("installed");
    let bg_req = SubagentRequest {
        prompt: "bg".into(),
        model: None,
        allowed_tools: vec![],
        max_turns: 10,
        timeout_secs: 300,
        subagent_type: None,
        run_in_background: true,
        cwd: None,
        isolation: None,
        provider_env: None,
    };
    let fg_req = SubagentRequest {
        run_in_background: false,
        ..bg_req.clone()
    };
    assert!(matches!(
        exec.classify(&bg_req),
        SubagentClassification::ExplicitBackground
    ));
    assert!(matches!(
        exec.classify(&fg_req),
        SubagentClassification::Foreground
    ));
}

// ---------------------------------------------------------------------------
// Test 4: AgentTool::execute with run_in_background: true returns a spawn
// marker JSON (NOT the real text) — preserves the TASK-AGS-104 contract for
// the background path.
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn agent_tool_background_returns_spawn_marker() {
    recording_executor();
    let tool = AgentTool::new();
    let input = json!({ "prompt": "do bg", "run_in_background": true });
    let result = tool.execute(input, &make_ctx()).await;
    assert!(!result.is_error, "unexpected error: {}", result.content);
    let v: serde_json::Value =
        serde_json::from_str(&result.content).expect("background path must return JSON");
    assert_eq!(v["status"], "spawned");
    assert!(v["agent_id"].is_string());
}

// ---------------------------------------------------------------------------
// Test 5: AgentTool::execute with run_in_background:false (default) calls
// run_to_completion on the installed executor. We prove this by checking
// the recording executor's `ran` flag AFTER execute returns — for the
// foreground path, execute must NOT return before run_to_completion was
// invoked at least once on the executor.
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn agent_tool_foreground_invokes_run_to_completion() {
    let exec = recording_executor();
    exec.ran.store(false, Ordering::SeqCst);
    exec.inner_completed.store(false, Ordering::SeqCst);
    let tool = AgentTool::new();
    let input = json!({ "prompt": "do fg", "run_in_background": false });

    let result = tool.execute(input, &make_ctx()).await;

    assert!(!result.is_error, "unexpected error: {}", result.content);
    assert_eq!(result.content, "recorded");
    assert!(exec.ran.load(Ordering::SeqCst));
    assert!(exec.inner_completed.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn task_create_prompt_defaults_request_fields() {
    let exec = recording_executor();
    *exec.last_request.lock().expect("clear request") = None;
    let result = TaskCreateTool
        .execute(
            json!({
                "subject": "Review",
                "description": "Review defaults",
                "prompt": "Review AGT-006"
            }),
            &make_ctx(),
        )
        .await;

    assert!(!result.is_error, "unexpected error: {}", result.content);
    let request = exec
        .last_request
        .lock()
        .expect("read request")
        .clone()
        .expect("foreground request recorded");
    assert_eq!(request.model, None);
    assert!(request.allowed_tools.is_empty());
    assert_eq!(request.max_turns, SubagentRequest::DEFAULT_MAX_TURNS);
    assert_eq!(request.timeout_secs, SubagentRequest::DEFAULT_TIMEOUT_SECS);
    assert_eq!(request.subagent_type, None);
    assert!(!request.run_in_background);
    assert_eq!(request.cwd, None);
    assert_eq!(request.isolation, None);
    assert_eq!(request.provider_env, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn task_create_foreground_propagates_request_fields() {
    let exec = recording_executor();
    *exec.last_request.lock().expect("clear request") = None;
    exec.last_nested.store(false, Ordering::SeqCst);
    let tool = TaskCreateTool;
    let input = json!({
        "subject": "Review",
        "description": "Review agent wiring",
        "prompt": "Review AGT-006",
        "model": "sonnet",
        "allowed_tools": ["Read", "Grep"],
        "subagent_type": "code-reviewer",
        "run_in_background": false,
        "cwd": "/tmp"
    });

    let result = tool.execute(input, &make_ctx()).await;

    assert!(!result.is_error, "unexpected error: {}", result.content);
    let response: serde_json::Value = serde_json::from_str(&result.content).expect("response json");
    assert!(response["task_id"].is_string());
    assert_eq!(response["result"], "recorded");
    assert!(exec.ran.load(Ordering::SeqCst));
    assert!(exec.last_nested.load(Ordering::SeqCst));
    let request = exec
        .last_request
        .lock()
        .expect("read request")
        .clone()
        .expect("foreground request recorded");
    assert_eq!(request.prompt, "Review AGT-006");
    assert_eq!(request.model.as_deref(), Some("sonnet"));
    assert_eq!(request.allowed_tools, ["Read", "Grep"]);
    assert_eq!(request.subagent_type.as_deref(), Some("code-reviewer"));
    assert!(!request.run_in_background);
    assert_eq!(request.cwd.as_deref(), Some("/tmp"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn task_create_background_classifies_propagated_request_fields() {
    let exec = recording_executor();
    *exec
        .last_classified_request
        .lock()
        .expect("clear classified request") = None;
    *exec.last_request.lock().expect("clear request") = None;
    exec.auto_bg_ms.store(1, Ordering::SeqCst);
    exec.run_delay_ms.store(50, Ordering::SeqCst);
    let tool = TaskCreateTool;
    let input = json!({
        "subject": "Review",
        "description": "Review in background",
        "prompt": "Review AGT-006",
        "model": "sonnet",
        "allowed_tools": ["Read", "Grep"],
        "subagent_type": "code-reviewer",
        "run_in_background": true,
        "cwd": "/tmp"
    });

    let result = tool.execute(input, &make_ctx()).await;

    assert!(!result.is_error, "unexpected error: {}", result.content);
    let response: serde_json::Value = serde_json::from_str(&result.content).expect("response json");
    assert!(response["task_id"].is_string());
    let task_id = response["task_id"].as_str().expect("task id").to_string();
    assert!(response["agent_id"].is_string());
    assert_eq!(response["status"], "spawned");
    let request = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let run_started = exec.run_notify.notified();
            if let Some(request) = exec.last_request.lock().expect("read request").clone()
                && request.prompt == "Review AGT-006"
            {
                break request;
            }
            run_started.await;
        }
    })
    .await
    .expect("TaskCreate background request must reach the installed executor");
    assert_eq!(request.prompt, "Review AGT-006");
    assert_eq!(request.model.as_deref(), Some("sonnet"));
    assert_eq!(request.allowed_tools, ["Read", "Grep"]);
    assert_eq!(request.subagent_type.as_deref(), Some("code-reviewer"));
    assert!(request.run_in_background);
    assert_eq!(request.cwd.as_deref(), Some("/tmp"));
    wait_for_task_status(&task_id, archon_tools::task_manager::TaskStatus::Completed).await;
    exec.auto_bg_ms.store(0, Ordering::SeqCst);
    exec.run_delay_ms.store(0, Ordering::SeqCst);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn task_create_auto_background_completes_tracked_task() {
    let exec = recording_executor();
    exec.auto_bg_ms.store(1, Ordering::SeqCst);
    exec.run_delay_ms.store(50, Ordering::SeqCst);
    let result = TaskCreateTool
        .execute(
            json!({
                "subject": "Review",
                "description": "Review after auto background",
                "prompt": "Review auto background"
            }),
            &make_ctx(),
        )
        .await;
    exec.auto_bg_ms.store(0, Ordering::SeqCst);
    exec.run_delay_ms.store(0, Ordering::SeqCst);

    assert!(!result.is_error, "unexpected error: {}", result.content);
    let response: serde_json::Value = serde_json::from_str(&result.content).expect("response json");
    assert_eq!(response["status"], "auto_backgrounded");
    let task_id = response["task_id"].as_str().expect("task id");
    wait_for_task_status(task_id, archon_tools::task_manager::TaskStatus::Completed).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn task_create_auto_background_fails_tracked_task() {
    let exec = recording_executor();
    exec.auto_bg_ms.store(1, Ordering::SeqCst);
    exec.run_delay_ms.store(50, Ordering::SeqCst);
    exec.fail.store(true, Ordering::SeqCst);
    let result = TaskCreateTool
        .execute(
            json!({
                "subject": "Review",
                "description": "Fail after auto background",
                "prompt": "Review auto background failure"
            }),
            &make_ctx(),
        )
        .await;
    exec.auto_bg_ms.store(0, Ordering::SeqCst);
    exec.run_delay_ms.store(0, Ordering::SeqCst);
    exec.fail.store(false, Ordering::SeqCst);

    assert!(!result.is_error, "unexpected error: {}", result.content);
    let response: serde_json::Value = serde_json::from_str(&result.content).expect("response json");
    assert_eq!(response["status"], "auto_backgrounded");
    let task_id = response["task_id"].as_str().expect("task id");
    wait_for_task_status(task_id, archon_tools::task_manager::TaskStatus::Failed).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn task_create_auto_background_sender_drop_fails_tracked_task() {
    let exec = recording_executor();
    exec.auto_bg_ms.store(1, Ordering::SeqCst);
    exec.run_delay_ms.store(50, Ordering::SeqCst);
    exec.panic.store(true, Ordering::SeqCst);
    let result = TaskCreateTool
        .execute(
            json!({
                "subject": "Review",
                "description": "Panic after auto background",
                "prompt": "Review auto background panic"
            }),
            &make_ctx(),
        )
        .await;
    exec.auto_bg_ms.store(0, Ordering::SeqCst);
    exec.run_delay_ms.store(0, Ordering::SeqCst);
    exec.panic.store(false, Ordering::SeqCst);

    assert!(!result.is_error, "unexpected error: {}", result.content);
    let response: serde_json::Value = serde_json::from_str(&result.content).expect("response json");
    assert_eq!(response["status"], "auto_backgrounded");
    let task_id = response["task_id"].as_str().expect("task id");
    wait_for_task_status(task_id, archon_tools::task_manager::TaskStatus::Failed).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn auto_background_ms_zero_disables_timer_arm() {
    recording_executor();
    let exec = get_subagent_executor().expect("installed");
    assert_eq!(
        exec.auto_background_ms(),
        0,
        "RecordingExecutor::auto_background_ms returns 0"
    );
    let tool = AgentTool::new();
    let input = json!({ "prompt": "fast fg", "run_in_background": false });
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tool.execute(input, &make_ctx()),
    )
    .await
    .expect("foreground execute must not hang with auto_bg=0");
    assert!(!result.is_error, "unexpected error: {}", result.content);
}
