//! Issue 324: the author stall pause hands a resume only the finding the
//! stalled authoring held, keeps it until a new attempt is recorded, and
//! pauses on an infrastructure fault instead of counting it as a defect.
use super::*;
use archon_workflow::{WorkflowAgentOutcome, WorkflowLlmClient};

/// What the scripted author does on every call.
#[derive(Clone, Copy)]
enum Reply {
    Script(&'static str),
    Transport,
    RecoverAfterTransport,
    /// The live agent client fails on the host's own I/O.
    Io,
    /// The live agent client finds the host's state damaged.
    Corrupt,
    /// The author answers with text that is no envelope.
    Garbage,
}

/// Gives `reply` to every call and records the task of each call.
struct ScriptedAuthor {
    reply: Reply,
    tasks: std::sync::Mutex<Vec<String>>,
}

impl ScriptedAuthor {
    fn new(reply: Reply) -> Arc<Self> {
        Arc::new(Self {
            reply,
            tasks: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn answer(&self, task: String) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.tasks.lock().unwrap().push(task);
        let script = match self.reply {
            Reply::Script(script) => script,
            Reply::RecoverAfterTransport => {
                if self.tasks.lock().unwrap().len() <= 4 {
                    return Err(WorkflowError::StageFailed(
                        "agent transport failed: connection reset by peer".into(),
                    ));
                }
                AUTHORED_DEMO_SCRIPT
            }
            Reply::Io => {
                return Err(WorkflowError::Io {
                    path: "/review/transcript".into(),
                    source: std::io::Error::other("disk on fire"),
                });
            }
            Reply::Corrupt => {
                return Err(WorkflowError::StateCorrupt("review: damaged record".into()));
            }
            Reply::Garbage => {
                return Ok(WorkflowAgentOutcome {
                    content: "this is {not json".into(),
                    tool_uses: vec![],
                    tokens_in: Some(1),
                    tokens_out: Some(1),
                    stop_reason: Some("end_turn".into()),
                });
            }
            Reply::Transport => {
                return Err(WorkflowError::StageFailed(
                    "agent transport failed: connection reset by peer".into(),
                ));
            }
        };
        let content = serde_json::json!({"status": "accepted", "summary": "authored",
            "evidence": [{"kind": "implementation", "summary": "authored script"}],
            "data": {"workflow_js": script}});
        Ok(WorkflowAgentOutcome {
            content: content.to_string(),
            tool_uses: vec![],
            tokens_in: Some(1),
            tokens_out: Some(1),
            stop_reason: Some("end_turn".into()),
        })
    }
}

#[async_trait::async_trait]
impl WorkflowLlmClient for ScriptedAuthor {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.answer(String::new())
    }
    async fn run_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.answer(call.task)
    }
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.answer(call.task)
    }
}

const WORKLESS: &str = r#"export const meta = { name: 'workless-demo', phases: [{ title: 'Only' }] }
export default async function workflow({ phase, log }) {
  await phase("No Real Work");
  await log("nothing spawned");
  return { accepted: [], blocked: [], notes: "did nothing" };
}
"#;

struct Fixture {
    _temp: tempfile::TempDir,
    store: WorkflowStore,
    run_id: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowStore::new(temp.path().join("workflows"));
        let run_id = store.create_run(test_spec()).expect("run").id;
        Self {
            _temp: temp,
            store,
            run_id,
        }
    }

    async fn author(
        &self,
        author: Arc<ScriptedAuthor>,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        let (ui_sink, _tui_rx) = default_workflow_ui_sink();
        let client =
            LiveV2AgentClient::new(author, ui_sink, Vec::new(), self.run_id.clone(), None, None);
        let spec = test_spec();
        WorkflowV2ScriptRunner::new(
            "author stall".to_string(),
            test_runtime(&spec),
            WorkflowV2AgentAdapter::new(),
            client,
            WorkflowV2ResultStore::new(self.store.run_dir(&self.run_id).join("v2")),
            self.store.clone(),
            self.run_id.clone(),
            true,
            None,
            None,
        )
        .run_authored_script_lifecycle(self.rejected().with_file_name("authored-workflow.js"))
        .await
    }

    fn rejected(&self) -> std::path::PathBuf {
        self.store.run_dir(&self.run_id).join("rejected-scripts")
    }

    fn control(&self, action: archon_workflow::LifecycleAction) {
        archon_workflow::LifecycleController::new(self.store.clone())
            .apply(&self.run_id, action)
            .unwrap();
    }

    fn status(&self) -> archon_workflow::RunStatus {
        self.store.load_state(&self.run_id).unwrap().status
    }

    /// The detail of the last `author_stall_pause` event.
    fn last_pause(&self) -> serde_json::Value {
        let events = std::fs::read_to_string(self.store.events_path(&self.run_id)).unwrap();
        let line = (events.lines().rev())
            .find(|line| line.contains("author_stall_pause"))
            .expect("the pause carries its evidence");
        let event: serde_json::Value = serde_json::from_str(line).unwrap();
        event.get("detail").cloned().unwrap_or(event)
    }

    /// Spends the defect budget on a script the pre-flight always rejects.
    async fn defect_stall(&self) {
        let error = self
            .author(ScriptedAuthor::new(Reply::Script(WORKLESS)))
            .await;
        assert!(
            matches!(error, Err(WorkflowError::ControlPaused(_))),
            "{error:?}"
        );
        assert!(self.rejected().join("stall-pause.json").exists());
    }
}

/// A provider outage recovers inside the workflow retry helper, with each
/// retry carrying the same pending call payload.
#[tokio::test]
async fn a_transport_outage_recovers_inside_the_workflow_helper() {
    let author = ScriptedAuthor::new(Reply::RecoverAfterTransport);
    let call = generic_agent_call();
    let expected_task = call.task.clone();
    archon_workflow::llm_retry::run_agent_with_transient_retry(
        &(author.clone() as Arc<dyn WorkflowLlmClient>),
        call,
        |_| async { Ok(()) },
    )
    .await
    .expect("the provider recovers");
    let tasks = author.tasks.lock().unwrap();
    assert_eq!(tasks.len(), 5);
    assert!(tasks.iter().all(|task| task == &expected_task));
}

/// Replaying a pending call during transport recovery preserves its payload.
#[tokio::test]
async fn a_transport_retries_preserve_the_pending_call_payload() {
    let author = ScriptedAuthor::new(Reply::RecoverAfterTransport);
    let call = generic_agent_call();
    let expected_task = call.task.clone();
    archon_workflow::llm_retry::run_agent_with_transient_retry(
        &(author.clone() as Arc<dyn WorkflowLlmClient>),
        call,
        |_| async { Ok(()) },
    )
    .await
    .expect("the provider recovers");
    let tasks = author.tasks.lock().unwrap();
    assert_eq!(tasks.len(), 5);
    assert!(tasks.iter().all(|task| task == &expected_task));
}

fn generic_agent_call() -> archon_workflow::WorkflowAgentCall {
    archon_workflow::WorkflowAgentCall {
        session_id: "AC-X-001".into(),
        task: "TASK-X-001".into(),
        cwd: None,
        ordinal: 1,
        attempt: 1,
        agent: archon_workflow::WorkflowAgentSpec {
            key: "AC-X-001".into(),
            display_name: "AC-X-001".into(),
            model: String::new(),
            phase: 1,
            critical: true,
            parallelizable: true,
            quality_threshold: 0.8,
            tool_access: archon_workflow::WorkflowAgentToolAccess::Full,
        },
        messages: Vec::new(),
        system: Vec::new(),
        tools: Vec::new(),
        allowed_tools: Vec::new(),
        timeout_secs: None,
        disable_auto_background: false,
        read_roots: Vec::new(),
        write_roots: Vec::new(),
        provider_env: None,
    }
}

/// Item 2: a resume stopped before its first attempt has a result keeps the
/// hand-over marker, so the next resume still gets the last finding.
#[tokio::test]
async fn a_resume_stopped_before_its_first_attempt_keeps_the_last_finding() {
    let run = Fixture::new();
    run.defect_stall().await;
    run.control(archon_workflow::LifecycleAction::Resume);
    run.control(archon_workflow::LifecycleAction::Pause);
    let stopped = run.author(ScriptedAuthor::new(Reply::Transport)).await;
    assert!(
        matches!(&stopped, Err(WorkflowError::ControlPaused(message))
            if !message.contains("transport")),
        "{stopped:?}"
    );
    assert!(run.rejected().join("stall-pause.json").exists());

    run.control(archon_workflow::LifecycleAction::Resume);
    let author = ScriptedAuthor::new(Reply::Script(AUTHORED_DEMO_SCRIPT));
    run.author(author.clone())
        .await
        .expect("the resume authors");
    let tasks = author.tasks.lock().unwrap();
    assert!(tasks[0].contains("ZERO agent calls"), "{}", tasks[0]);
    assert!(
        !run.rejected().join("stall-pause.json").exists(),
        "the marker goes once the first attempt's result is recorded"
    );
}

/// Item 3: an I/O fault in the run store during the authoring call (here the
/// call records cannot be written) is not an authoring defect. It pauses the
/// run at once with the fault as evidence; no rejection is recorded and
/// nothing is fed to an author.
#[tokio::test]
async fn an_io_fault_in_authoring_pauses_at_once_and_is_not_a_defect() {
    let run = Fixture::new();
    let v2 = run.store.run_dir(&run.run_id).join("v2");
    std::fs::write(&v2, b"not a directory").unwrap();
    let author = ScriptedAuthor::new(Reply::Script(AUTHORED_DEMO_SCRIPT));
    let result = run.author(author.clone()).await;
    assert!(
        matches!(&result, Err(WorkflowError::ControlPaused(message))
            if message.contains("infrastructure fault") && message.contains("io error")),
        "{result:?}"
    );
    assert_eq!(run.status(), archon_workflow::RunStatus::Paused);
    let detail = run.last_pause();
    assert_eq!(detail["stall"], "infrastructure_fault", "{detail}");
    assert_eq!(detail["defect_attempts"], 0);
    assert!(!run.rejected().join("attempt-1.json").exists());
}

/// Item 3: a rejection record that cannot be written (an I/O fault in the
/// run store) pauses the run with its evidence instead of leaving it Running
/// behind an error.
#[tokio::test]
async fn a_rejection_record_that_cannot_be_written_pauses_the_run() {
    let run = Fixture::new();
    std::fs::write(run.rejected(), b"not a directory").unwrap();
    let result = run
        .author(ScriptedAuthor::new(Reply::Script(WORKLESS)))
        .await;
    assert!(
        matches!(&result, Err(WorkflowError::ControlPaused(message))
            if message.contains("infrastructure fault")),
        "{result:?}"
    );
    assert_eq!(run.status(), archon_workflow::RunStatus::Paused);
    assert_eq!(run.last_pause()["stall"], "infrastructure_fault");
}

/// Item 3: a persisted script that cannot be read (an I/O fault) pauses the
/// run with its evidence instead of failing it.
#[tokio::test]
async fn a_persisted_script_that_cannot_be_read_pauses_the_run() {
    let run = Fixture::new();
    std::fs::create_dir_all(run.rejected().with_file_name("authored-workflow.js")).unwrap();
    let result = run
        .author(ScriptedAuthor::new(Reply::Script(AUTHORED_DEMO_SCRIPT)))
        .await;
    assert!(
        matches!(&result, Err(WorkflowError::ControlPaused(message))
            if message.contains("infrastructure fault")),
        "{result:?}"
    );
    assert_eq!(run.status(), archon_workflow::RunStatus::Paused);
    assert_eq!(run.last_pause()["stall"], "infrastructure_fault");
}

/// Round 2: a run-store I/O fault inside the author call's dispatch (its
/// transport evidence file cannot be opened) is not re-typed as a failed
/// stage: it pauses at once as an infrastructure fault, records no rejection
/// and is never handed to an author.
#[tokio::test]
async fn a_dispatch_store_io_fault_pauses_at_once_and_is_never_a_rejection() {
    let run = Fixture::new();
    let v2 = run.store.run_dir(&run.run_id).join("v2");
    std::fs::create_dir_all(v2.join("transport.jsonl")).unwrap();
    let author = ScriptedAuthor::new(Reply::Script(AUTHORED_DEMO_SCRIPT));
    let result = run.author(author.clone()).await;
    assert!(
        matches!(&result, Err(WorkflowError::ControlPaused(message))
            if message.contains("infrastructure fault") && message.contains("transport.jsonl")),
        "{result:?}"
    );
    assert!(!run.rejected().join("attempt-1.json").exists());
    let detail = run.last_pause();
    assert_eq!(detail["stall"], "infrastructure_fault", "{detail}");
    assert_eq!(detail["cause"], "infrastructure_fault");
    assert_eq!(detail["defect_attempts"], 0);
}

/// Round 2: an I/O fault the live agent client raises is the host's, not a
/// provider drop: no transport retry, no rejection, a pause at once.
#[tokio::test]
async fn an_agent_client_io_fault_pauses_at_once_and_is_not_transport() {
    let run = Fixture::new();
    let author = ScriptedAuthor::new(Reply::Io);
    let result = run.author(author.clone()).await;
    assert!(
        matches!(&result, Err(WorkflowError::ControlPaused(message))
            if message.contains("infrastructure fault") && message.contains("disk on fire")),
        "{result:?}"
    );
    assert_eq!(author.tasks.lock().unwrap().len(), 1, "never retried");
    assert!(!run.rejected().join("attempt-1.json").exists());
    assert_eq!(run.last_pause()["stall"], "infrastructure_fault");
}

/// Round 2: a damaged store the live agent client reports is kept as such.
#[tokio::test]
async fn an_agent_client_state_corrupt_fault_pauses_at_once() {
    let run = Fixture::new();
    let author = ScriptedAuthor::new(Reply::Corrupt);
    let result = run.author(author.clone()).await;
    assert!(
        matches!(&result, Err(WorkflowError::ControlPaused(message))
            if message.contains("infrastructure fault") && message.contains("damaged record")),
        "{result:?}"
    );
    assert_eq!(author.tasks.lock().unwrap().len(), 1);
    assert_eq!(run.last_pause()["stall"], "infrastructure_fault");
}

/// Round 2 guard: author output that is no envelope stays an authoring
/// defect, handed back to the author.
#[tokio::test]
async fn unparseable_author_output_stays_a_defect() {
    let run = Fixture::new();
    let result = run.author(ScriptedAuthor::new(Reply::Garbage)).await;
    assert!(
        matches!(&result, Err(WorkflowError::ControlPaused(_))),
        "{result:?}"
    );
    assert_eq!(run.last_pause()["stall"], "defect_attempts_exhausted");
    assert!(run.rejected().join("attempt-6.json").exists());
}

/// Round 2 (a): a death between persisting the script and removing the
/// hand-over marker leaves a matching marker. The persisted-script path
/// removes it, so a later re-author is not handed that old finding.
#[tokio::test]
async fn a_persisted_script_removes_a_marker_left_by_a_death_after_persisting() {
    let run = Fixture::new();
    run.defect_stall().await;
    let authored = run.rejected().with_file_name("authored-workflow.js");
    std::fs::write(&authored, AUTHORED_DEMO_SCRIPT.trim()).unwrap();
    run.control(archon_workflow::LifecycleAction::Resume);
    run.author(ScriptedAuthor::new(Reply::Script(AUTHORED_DEMO_SCRIPT)))
        .await
        .expect("the persisted script executes");
    assert!(
        !run.rejected().join("stall-pause.json").exists(),
        "the persisted script supersedes the marker"
    );
}
