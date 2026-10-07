//! Real spawned executor and tool-registry polls retain their captured owner.
use super::*;
use archon_tools::{subagent_executor::*, subagent_request::SubagentRequest, tool::*};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Barrier {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    admitted: AtomicUsize,
}
struct Executor(Arc<Barrier>);
#[async_trait::async_trait]
impl SubagentExecutor for Executor {
    async fn run_to_completion(
        &self,
        _: String,
        _: SubagentRequest,
        _: ToolContext,
        _: tokio_util::sync::CancellationToken,
    ) -> Result<String, ExecutorError> {
        self.0.entered.notify_one();
        self.0.release.notified().await;
        self.0.admitted.fetch_add(1, Ordering::SeqCst);
        Ok("dispatched".into())
    }
    async fn on_inner_complete(&self, _: String, _: Result<String, String>) {}
    async fn on_visible_complete(
        &self,
        _: String,
        _: Result<String, String>,
        _: bool,
    ) -> OutcomeSideEffects {
        OutcomeSideEffects::default()
    }
    fn auto_background_ms(&self) -> u64 {
        0
    }
    fn classify(&self, _: &SubagentRequest) -> SubagentClassification {
        SubagentClassification::Foreground
    }
}
struct BlockedTool(Arc<Barrier>, PermissionLevel);
#[async_trait::async_trait]
impl Tool for BlockedTool {
    fn capability(&self) -> ToolCapability {
        ToolCapability::HostLocal
    }
    fn name(&self) -> &str {
        "Probe"
    }
    fn description(&self) -> &str {
        "Admission probe"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    fn permission_level(&self, _: &serde_json::Value) -> PermissionLevel {
        self.1
    }
    async fn execute(&self, _: serde_json::Value, _: &ToolContext) -> ToolResult {
        self.0.entered.notify_one();
        self.0.release.notified().await;
        self.0.admitted.fetch_add(1, Ordering::SeqCst);
        ToolResult::success("executed")
    }
}
/// Bound every wait: a regression must fail the child, never hang it.
async fn bounded<T>(what: &str, work: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(60), work)
        .await
        .unwrap_or_else(|_| panic!("{what} did not happen within 60 s"))
}

/// What a spawned path reported: kept (with its text), a control stop, or an
/// ordinary failure.
#[derive(Debug, PartialEq)]
enum Seen {
    Kept,
    Stopped,
    Failed(String),
}

async fn probe(case: &str) {
    let (temp, store, id) = new_run();
    set_status(&store, &id, RunStatus::Running);
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&id).join("v2"));
    v2.bind_session_executor(store.load_state(&id).unwrap().generation);
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        run_store: Some(
            crate::command::workflow_live::workflow_live_v2::run_store_scope(Some(&v2), None, None),
        ),
        ..Default::default()
    };
    let barrier = Arc::new(Barrier::default());
    let provider = case.starts_with("provider");
    if provider {
        install_subagent_executor(Arc::new(Executor(barrier.clone())));
    }
    let dispatch = |ctx: ToolContext| {
        let barrier = barrier.clone();
        let level = match case {
            "tool_risky" => PermissionLevel::Risky,
            "tool_dangerous" => PermissionLevel::Dangerous,
            _ => PermissionLevel::Safe,
        };
        tokio::spawn(async move {
            if provider {
                let request: SubagentRequest = serde_json::from_value(serde_json::json!({
                    "prompt":"inspect", "max_turns":100, "timeout_secs":86400
                }))
                .unwrap();
                match archon_tools::agent_tool::run_subagent_foreground_with_system(
                    "owned-execution".into(),
                    request,
                    Vec::new(),
                    Default::default(),
                    ctx,
                )
                .await
                {
                    SubagentOutcome::Completed(_) => Seen::Kept,
                    SubagentOutcome::Cancelled => Seen::Stopped,
                    other => Seen::Failed(format!("{other:?}")),
                }
            } else {
                let mut registry = archon_core::dispatch::ToolRegistry::new();
                registry.register(Box::new(BlockedTool(barrier, level)));
                let result = registry
                    .dispatch("Probe", serde_json::json!({}), &ctx)
                    .await;
                match (result.is_error, result.content.as_str()) {
                    (false, _) => Seen::Kept,
                    (true, text) if text.contains("workflow run control:") => Seen::Stopped,
                    (true, text) => Seen::Failed(text.to_string()),
                }
            }
        })
    };
    let work = dispatch(ctx.clone());
    bounded(
        "admission of the first dispatch",
        barrier.entered.notified(),
    )
    .await;
    assert_eq!(
        barrier.admitted.load(Ordering::SeqCst),
        0,
        "named await reached"
    );
    let ctl = archon_workflow::LifecycleController::new(store);
    let superseded = !matches!(case, "provider_pause" | "provider_cancel" | "tool_pause");
    if case == "provider_cancel" {
        ctl.apply(&id, archon_workflow::LifecycleAction::Cancel)
            .unwrap();
    } else {
        ctl.apply(&id, archon_workflow::LifecycleAction::Pause)
            .unwrap();
        if superseded {
            ctl.apply(&id, archon_workflow::LifecycleAction::Resume)
                .unwrap();
        }
    }
    barrier.release.notify_one();
    let seen = bounded("the first dispatch's end", work).await.unwrap();
    if superseded {
        // A new executor owns the run: the old owner's work is a typed stop,
        // never a failure, and it did not go on.
        assert_eq!(seen, Seen::Stopped, "{case}");
        assert_eq!(barrier.admitted.load(Ordering::SeqCst), 0, "no progress");
    } else {
        // Admitted before a pause or cancel of its own owner: the work that
        // finished before the fence looked is kept; otherwise it is a typed
        // stop. Which one depends on when the fence looks (the deterministic
        // order is covered by `archon_tools` admission tests); never a failure.
        let finished = barrier.admitted.load(Ordering::SeqCst) == 1;
        match seen {
            Seen::Kept => assert!(finished, "{case}: a kept result finished"),
            Seen::Stopped => {}
            Seen::Failed(text) => panic!("{case}: a pause is never a failure: {text}"),
        }
    }
    // Nothing new is admitted after the stop: a second dispatch never enters.
    let second = bounded("the refused second dispatch", dispatch(ctx))
        .await
        .unwrap();
    assert_eq!(
        second,
        Seen::Stopped,
        "{case}: a new admission is a typed stop"
    );
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            barrier.entered.notified()
        )
        .await
        .is_err(),
        "{case}: the second dispatch never entered its executor or tool"
    );
}

// Executor installation is process-global. Each case uses a fresh test
// process so other provider tests cannot replace its executor at the barrier.
#[test]
#[ignore = "invoked by the isolated regression cases"]
fn round3_spawned_admission_child() {
    let case = std::env::var("ARCHON_R3_ADMISSION_CASE").unwrap();
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(240), probe(&case))
                .await
                .expect("the probe must end; a hang is a failure")
        });
}
fn child(case: &str) {
    let output = archon_shell::spawn::command(std::env::current_exe().unwrap())
        .args(["--ignored", "round3_spawned_admission_child"])
        .env("ARCHON_R3_ADMISSION_CASE", case)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{case}: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
#[test]
fn round3_spawned_provider_resume() {
    child("provider_resume");
}
#[test]
fn round3_spawned_provider_pause() {
    child("provider_pause");
}
#[test]
fn round3_spawned_provider_cancel() {
    child("provider_cancel");
}
#[test]
fn round3_spawned_safe_tool() {
    child("tool_safe");
}
#[test]
fn round3_spawned_risky_tool() {
    child("tool_risky");
}
#[test]
fn round3_spawned_dangerous_tool() {
    child("tool_dangerous");
}
#[test]
fn round4_spawned_tool_pause_is_a_stop_not_a_tool_failure() {
    child("tool_pause");
}
