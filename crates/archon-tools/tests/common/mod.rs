//! Recording executor shared by the TASK-AGS-105 test binaries.
//!
//! Lives in `tests/common/` so cargo treats it as a module rather than a test
//! binary of its own. Each binary that includes it uses only part of it, hence
//! the blanket `dead_code` allow: the alternative is a per-item allow list that
//! has to be edited every time a test moves between the two files.

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::Notify;

use archon_tools::agent_tool::SubagentRequest;
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
    install_subagent_executor,
};
use archon_tools::tool::ToolContext;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

pub struct RecordingExecutor {
    pub ran: AtomicBool,
    pub visible_completed: AtomicBool,
    pub inner_completed: AtomicBool,
    pub auto_bg_ms: AtomicU64,
    pub run_delay_ms: AtomicU64,
    pub fail: AtomicBool,
    pub panic: AtomicBool,
    pub last_request: Mutex<Option<SubagentRequest>>,
    pub last_classified_request: Mutex<Option<SubagentRequest>>,
    pub last_nested: AtomicBool,
    pub run_count: AtomicUsize,
    pub run_notify: Notify,
}

impl RecordingExecutor {
    pub fn new() -> Self {
        Self {
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
        }
    }
}

impl Default for RecordingExecutor {
    fn default() -> Self {
        Self::new()
    }
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

pub fn recording_executor() -> Arc<RecordingExecutor> {
    static EXECUTOR: OnceLock<Arc<RecordingExecutor>> = OnceLock::new();
    let exec = EXECUTOR
        .get_or_init(|| Arc::new(RecordingExecutor::new()))
        .clone();
    install_subagent_executor(exec.clone());
    exec
}

/// TaskCreate appends `AUTONOMY_RULE` to every delegated prompt, so the caller's
/// text reaches the executor as a prefix rather than as the whole string.
pub fn assert_prompt_propagated(prompt: &str) {
    assert!(
        prompt.starts_with("Review AGT-006"),
        "caller prompt must reach the executor verbatim as a prefix: {prompt}"
    );
    assert!(
        prompt.contains("AUTONOMOUS EXECUTION"),
        "the autonomy rule must be appended to a delegated prompt: {prompt}"
    );
}

pub fn make_ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "task-ags-105-test".into(),
        ..Default::default()
    }
}

pub async fn wait_for_task_status(task_id: &str, expected: archon_tools::task_manager::TaskStatus) {
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
