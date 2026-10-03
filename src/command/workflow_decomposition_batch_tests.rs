use super::*;
struct BatchLlm {
    active: AtomicUsize,
    peak: AtomicUsize,
}
#[async_trait::async_trait]
impl WorkflowLlmClient for BatchLlm {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!()
    }
    async fn run_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        assert!(
            archon_tools::read_boundary::current().contains(&".archon".into()),
            "raw dispatch lost host exclusion policy"
        );
        let n = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(n, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        let id = call
            .task
            .lines()
            .find_map(|line| line.strip_prefix("Author ONLY entry "))
            .and_then(|line| line.split(':').next())
            .unwrap_or("unused");
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({"id":id}).to_string(),
            stop_reason: Some("end_turn".into()),
            ..Default::default()
        })
    }
}
#[tokio::test]
async fn fixed_author_pool_overlaps_through_real_quickjs_host_and_keeps_read_policy() {
    let temp = tempfile::tempdir().unwrap();
    let spec = test_spec();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(spec.clone()).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let (sink, _rx) = default_workflow_ui_sink();
    let llm = Arc::new(BatchLlm {
        active: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
    });
    let client = LiveV2AgentClient::new(llm.clone(), sink, vec![], run.id.clone(), None, Some(10))
        .with_fixed_raw_tool_policy(vec!["Read".into(), "Grep".into(), "Glob".into()]);
    let criteria = (1..=7)
        .map(|n| (format!("AC-X-{n:03}"), format!("criterion {n}")))
        .collect::<std::collections::BTreeMap<_, _>>();
    let summary = WorkflowV2ScriptRunner::new("batch proof".into(),test_runtime(&spec),WorkflowV2AgentAdapter::new(),client,v2,store,run.id,true,None,
        Some(serde_json::json!({"projectRoot":temp.path(),"repositoryRoot":temp.path(),"prdPath":temp.path().join("prd.md"),"prdDigest":"a".repeat(64),"taskRoot":temp.path().join("tasks"),"gateMode":"observe","acceptanceCriteria":criteria,"authorMaxParallelism":3})))
        .with_raw_outcomes(true).with_host_command_executor(Arc::new(PhaseHost {calls:Mutex::new(vec![])}))
        .run(crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE).await.unwrap();
    assert_eq!(summary.status, WorkflowV2Status::Accepted);
    assert_eq!(llm.peak.load(Ordering::SeqCst), 3);
    assert_eq!(llm.active.load(Ordering::SeqCst), 0);
}

/// The script-level schedule proofs (virtual clock): pool and prefix-window
/// makespans, in-flight cap, index-only acceptance prompts, failure and stop
/// semantics (Issue-247).
#[test]
fn bounded_author_pool_schedule_prompts_and_failures() {
    let output = std::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/command/workflow_decompose_pool_test.cjs"
        ))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Acceptance authors driven through the real QuickJS host (Issue-247). The
/// first entry holds until its two window siblings have ended; the script
/// must not start the fourth entry before the first one ends, and a pause
/// written mid-pool must stop new dispatch.
struct WindowLlm {
    events: Mutex<Vec<(&'static str, String, String)>>,
    active: AtomicUsize,
    peak: AtomicUsize,
    pause: Option<(WorkflowStore, String)>,
    paused: std::sync::atomic::AtomicBool,
    started_after_pause: AtomicUsize,
}

impl WindowLlm {
    fn new(pause: Option<(WorkflowStore, String)>) -> Self {
        Self {
            events: Mutex::new(vec![]),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            pause,
            paused: Default::default(),
            started_after_pause: AtomicUsize::new(0),
        }
    }

    fn seen(&self, kind: &str, ids: &[&str]) -> bool {
        let events = self.events.lock().unwrap();
        ids.iter()
            .all(|id| events.iter().any(|(k, i, _)| *k == kind && i == id))
    }

    /// Gate: hold until every call in `ids` has a `kind` event. The bound
    /// only turns a scheduler that never gets there into a failure, not a hang.
    async fn wait_for(&self, kind: &str, ids: &[&str]) {
        for _ in 0..2000 {
            if self.seen(kind, ids) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("calls {ids:?} never had a {kind} event");
    }

    fn position(&self, kind: &str, id: &str) -> usize {
        let events = self.events.lock().unwrap();
        events
            .iter()
            .position(|(k, i, _)| *k == kind && i == id)
            .unwrap_or_else(|| panic!("no {kind} event for {id}: {events:?}"))
    }
}

#[async_trait::async_trait]
impl WorkflowLlmClient for WindowLlm {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!()
    }
    async fn run_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let id = call
            .task
            .lines()
            .find_map(|line| line.strip_prefix("Author ONLY entry "))
            .and_then(|line| line.split(':').next())
            .unwrap_or("unused")
            .to_string();
        let prior = call
            .task
            .split("Previously completed entries: ")
            .nth(1)
            .unwrap_or("[]")
            .to_string();
        if self.paused.load(Ordering::SeqCst) {
            self.started_after_pause.fetch_add(1, Ordering::SeqCst);
        }
        let n = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(n, Ordering::SeqCst);
        self.events
            .lock()
            .unwrap()
            .push(("start", id.clone(), prior));
        match (id.as_str(), &self.pause) {
            // The first window is held open until all three of its calls have
            // started, so the peak of 3 is a gate, not a race between sleeps.
            ("AC-X-002" | "AC-X-003", None) => {
                self.wait_for("start", &["AC-X-001", "AC-X-002", "AC-X-003"])
                    .await;
            }
            ("AC-X-001", None) => {
                self.wait_for("end", &["AC-X-002", "AC-X-003"]).await;
                // Room for a barrier-free scheduler to start AC-X-004 early.
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            ("AC-X-002", Some((store, run_id))) => {
                self.wait_for("end", &["AC-X-001", "AC-X-003", "AC-X-004"])
                    .await;
                archon_workflow::LifecycleController::new(store.clone())
                    .apply(run_id, archon_workflow::LifecycleAction::Pause)
                    .expect("pause the run mid-pool");
                self.paused.store(true, Ordering::SeqCst);
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(5)).await,
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.events
            .lock()
            .unwrap()
            .push(("end", id.clone(), String::new()));
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({"id":id}).to_string(),
            stop_reason: Some("end_turn".into()),
            ..Default::default()
        })
    }
}

async fn run_window(
    with_pause: bool,
) -> (
    Arc<WindowLlm>,
    archon_workflow::WorkflowResult<WorkflowV2ScriptSummary>,
) {
    let temp = tempfile::tempdir().unwrap();
    let spec = test_spec();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(spec.clone()).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let (sink, _rx) = default_workflow_ui_sink();
    let llm = Arc::new(WindowLlm::new(
        with_pause.then(|| (store.clone(), run.id.clone())),
    ));
    let client = LiveV2AgentClient::new(llm.clone(), sink, vec![], run.id.clone(), None, Some(10))
        .with_fixed_raw_tool_policy(vec!["Read".into(), "Grep".into(), "Glob".into()]);
    let criteria = (1..=7)
        .map(|n| (format!("AC-X-{n:03}"), format!("criterion {n}")))
        .collect::<std::collections::BTreeMap<_, _>>();
    let result = WorkflowV2ScriptRunner::new("window proof".into(),test_runtime(&spec),WorkflowV2AgentAdapter::new(),client,v2,store,run.id,true,None,
        Some(serde_json::json!({"projectRoot":temp.path(),"repositoryRoot":temp.path(),"prdPath":temp.path().join("prd.md"),"prdDigest":"a".repeat(64),"taskRoot":temp.path().join("tasks"),"gateMode":"observe","acceptanceCriteria":criteria,"authorMaxParallelism":3})))
        .with_raw_outcomes(true).with_host_command_executor(Arc::new(PhaseHost {calls:Mutex::new(vec![])}))
        .run(crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE).await;
    (llm, result)
}

#[tokio::test]
async fn fixed_acceptance_window_holds_only_calls_behind_a_slow_entry() {
    let (llm, result) = run_window(false).await;
    assert_eq!(result.unwrap().status, WorkflowV2Status::Accepted);
    assert_eq!(llm.peak.load(Ordering::SeqCst), 3);
    let first_end = llm.position("end", "AC-X-001");
    assert!(llm.position("start", "AC-X-002") < first_end);
    assert!(llm.position("start", "AC-X-003") < first_end);
    assert!(
        llm.position("start", "AC-X-004") > first_end,
        "entry 4 started inside the window of the unfinished entry 1"
    );
    let events = llm.events.lock().unwrap();
    let prior = |id: &str| -> Vec<String> {
        let (_, _, prior) = events
            .iter()
            .find(|(k, i, _)| *k == "start" && i == id)
            .unwrap();
        serde_json::from_str::<Vec<serde_json::Value>>(prior)
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert!(prior("AC-X-003").is_empty());
    assert_eq!(prior("AC-X-004"), ["AC-X-001"]);
    assert_eq!(
        prior("AC-X-007"),
        ["AC-X-001", "AC-X-002", "AC-X-003", "AC-X-004"]
    );
}

/// End-to-end guarantee: once a pause is written mid-pool, no model call
/// starts and the caller sees the pause. Scope: this cannot catch a script
/// that dispatches after a stop, because the host polls run control before
/// every dispatch (`workflow_live_v2_script_host_exec.rs`) and refuses such a
/// call before anything observable happens. The script's own stop rules (no
/// launch after a failure or a thrown stop, queued launches skipped, started
/// calls settled, a stop outranking a failed reply) are proved by
/// `workflow_decompose_pool_test.cjs` and `workflow_decompose_cost_test.cjs`,
/// which are the authority for them.
#[tokio::test]
async fn fixed_acceptance_pause_mid_pool_stops_new_dispatch() {
    let (llm, result) = run_window(true).await;
    let error = result.expect_err("a paused run must unwind the script");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "the caller sees the pause: {error:?}"
    );
    assert_eq!(
        llm.started_after_pause.load(Ordering::SeqCst),
        0,
        "no author call may start after the pause"
    );
    assert!(llm.peak.load(Ordering::SeqCst) <= 3);
}
