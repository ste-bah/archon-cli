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
    let (work, expected): (tokio::task::JoinHandle<bool>, usize) = if case.starts_with("provider") {
        let expected = 0;
        install_subagent_executor(Arc::new(Executor(barrier.clone())));
        let request: SubagentRequest = serde_json::from_value(serde_json::json!({
            "prompt":"inspect", "max_turns":100, "timeout_secs":86400
        }))
        .unwrap();
        (
            tokio::spawn(async move {
                matches!(
                    archon_tools::agent_tool::run_subagent_foreground_with_system(
                        "owned-execution".into(),
                        request,
                        Vec::new(),
                        Default::default(),
                        ctx,
                    )
                    .await,
                    SubagentOutcome::Failed(_)
                )
            }),
            expected,
        )
    } else {
        let level = match case {
            "tool_risky" => PermissionLevel::Risky,
            "tool_dangerous" => PermissionLevel::Dangerous,
            _ => PermissionLevel::Safe,
        };
        let mut registry = archon_core::dispatch::ToolRegistry::new();
        registry.register(Box::new(BlockedTool(barrier.clone(), level)));
        (
            tokio::spawn(async move {
                registry
                    .dispatch("Probe", serde_json::json!({}), &ctx)
                    .await
                    .is_error
            }),
            0,
        )
    };
    barrier.entered.notified().await;
    assert_eq!(
        barrier.admitted.load(Ordering::SeqCst),
        expected,
        "named await reached"
    );
    let ctl = archon_workflow::LifecycleController::new(store);
    if case == "provider_cancel" {
        ctl.apply(&id, archon_workflow::LifecycleAction::Cancel)
            .unwrap();
    } else {
        ctl.apply(&id, archon_workflow::LifecycleAction::Pause)
            .unwrap();
        if case != "provider_pause" {
            ctl.apply(&id, archon_workflow::LifecycleAction::Resume)
                .unwrap();
        }
    }
    barrier.release.notify_one();
    assert!(
        work.await.unwrap(),
        "the spawned path must refuse its old owner"
    );
    assert_eq!(
        barrier.admitted.load(Ordering::SeqCst),
        expected,
        "no subsequent admission"
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
        .block_on(probe(&case));
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
