//! The message router's resume, through the process executor, while the
//! executor's capacity is taken: the resume belongs to its run, and the
//! caller's interrupt stops it while it queues.
#[path = "support/boundary_harness.rs"]
mod harness;
#[path = "support/resume_memory_harness.rs"]
mod memory_harness;
use archon_core::agent::AgentConfig;
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
};
use archon_tools::subagent_request::SubagentRequest;
use archon_tools::tool::ToolContext;
use harness::*;
use memory_harness::*;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// The process executor is one slot; these tests take turns with it.
static EXECUTOR: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The real executor, with the auto-background timer set by the test.
struct Timed(Arc<archon_core::subagent_executor::AgentSubagentExecutor>, u64);
#[async_trait::async_trait]
impl SubagentExecutor for Timed {
    async fn run_to_completion(
        &self,
        id: String,
        request: SubagentRequest,
        ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        self.0.run_to_completion(id, request, ctx, cancel).await
    }
    async fn run_to_completion_with_system(
        &self,
        id: String,
        request: SubagentRequest,
        system: Vec<serde_json::Value>,
        ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        self.0
            .run_to_completion_with_system(id, request, system, ctx, cancel)
            .await
    }
    async fn on_inner_complete(&self, id: String, result: Result<String, String>) {
        self.0.on_inner_complete(id, result).await
    }
    async fn on_visible_complete(
        &self,
        id: String,
        result: Result<String, String>,
        nested: bool,
    ) -> OutcomeSideEffects {
        self.0.on_visible_complete(id, result, nested).await
    }
    fn auto_background_ms(&self) -> u64 {
        self.1
    }
    fn classify(&self, request: &SubagentRequest) -> SubagentClassification {
        self.0.classify(request)
    }
}

/// One slot of capacity, a stopped `child` spawned under a workflow context,
/// and a `holder` run that keeps the slot for `hold_ms`.
async fn with_slot_taken(
    session: &str,
    hold_ms: u64,
    auto_background_ms: u64,
) -> (tempfile::TempDir, std::path::PathBuf, Host, tokio::task::JoinHandle<()>) {
    let (temp, root) = real_temp();
    let workspace = dir(&root, "workspace");
    let host = Host::with_config(
        &root,
        session,
        vec![
            STOP,
            ("Wait", serde_json::json!({"ms": hold_ms})),
            STOP,
            ("ContextProbe", serde_json::json!({})),
            STOP,
        ],
        AgentConfig {
            max_subagent_concurrency: 1,
            ..Default::default()
        },
    );
    archon_tools::subagent_executor::install_subagent_executor(Arc::new(Timed(
        host.executor.clone(),
        auto_background_ms,
    )));
    let mut spawn = request(&workspace, None, vec![]);
    spawn.allowed_tools.push("ContextProbe".into());
    host.spawn(
        "child",
        spawn,
        ToolContext {
            denied_directory_names: vec!["secret".into()],
            run_store: Some(Default::default()),
            ..parent(&root, &[])
        },
    )
    .await
    .unwrap();
    history(&store(&root), "child");
    let executor = host.executor.clone();
    let mut hold = request(&workspace, None, vec![]);
    hold.allowed_tools = vec!["Wait".into()];
    let session_id = host.session.clone();
    let holder = tokio::spawn(async move {
        executor
            .run_to_completion(
                "holder".into(),
                hold,
                ToolContext {
                    session_id,
                    ..ToolContext::default()
                },
                CancellationToken::new(),
            )
            .await
            .unwrap();
    });
    while host.turns() < 2 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    (temp, root, host, holder)
}

async fn resume(host: &Host, root: &std::path::Path, cancel: CancellationToken) -> String {
    let result = archon_core::agents::transcript::resume_agent(
        &store(root),
        &host.manager,
        "child",
        "continue",
        ToolContext {
            session_id: host.session.clone(),
            ..parent(root, &[])
        },
        cancel,
    )
    .await;
    format!("{}{}", if result.is_error { "error: " } else { "" }, result.content)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_auto_backgrounded_resume_still_runs_with_its_stored_context() {
    let _turn = EXECUTOR.lock().await;
    let (_t, root, host, holder) = with_slot_taken("router-background", 400, 50).await;
    let answer = resume(&host, &root, CancellationToken::new()).await;
    assert!(answer.contains("auto-backgrounded"), "{answer}");
    holder.await.unwrap();
    while host.turns() < 5 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let contexts = host.contexts.lock().unwrap();
    let resumed = contexts.last().expect("the resumed agent never ran");
    assert_eq!(
        resumed.denied_directory_names,
        vec!["secret".to_string()],
        "the resume ran as a new spawn without its stored context"
    );
    assert!(resumed.run_store.is_some());
    assert!(
        host.last_messages()
            .iter()
            .any(|message| message["content"] == "done"),
        "the resume ran without its history"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_callers_interrupt_stops_a_resume_queued_for_capacity() {
    let _turn = EXECUTOR.lock().await;
    let (_t, root, host, holder) = with_slot_taken("router-interrupt", 800, 0).await;
    let caller = CancellationToken::new();
    let interrupt = caller.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        interrupt.cancel();
    });
    let started = std::time::Instant::now();
    let answer = resume(&host, &root, caller.child_token()).await;
    assert!(
        started.elapsed() < Duration::from_millis(600),
        "the interrupt waited for capacity: {answer}"
    );
    assert_eq!(answer, "error: Error: subagent cancelled", "not reported as cancelled");
    holder.await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(host.turns(), 3, "the interrupted resume still ran");
}
