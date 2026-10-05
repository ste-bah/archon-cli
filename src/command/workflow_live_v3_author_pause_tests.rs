//! Issue 296: authoring attempts spent without an accepted script pause the
//! run with their evidence; they never fail it. The resume authors again with
//! a new agent, handed the last finding, and the run goes on.
use super::*;
use archon_workflow::{WorkflowAgentOutcome, WorkflowLlmClient};

/// Replies `script` as the authored workflow to every call, and records for
/// each call whether it continued a session and the task it was given.
struct RecordingAuthor {
    script: String,
    calls: std::sync::Mutex<Vec<(bool, String)>>,
}

impl RecordingAuthor {
    fn new(script: &str) -> Arc<Self> {
        Arc::new(Self {
            script: script.to_string(),
            calls: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn reply(&self) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let content = serde_json::json!({"status": "accepted", "summary": "authored",
            "evidence": [{"kind": "implementation", "summary": "authored script"}],
            "data": {"workflow_js": self.script}});
        Ok(WorkflowAgentOutcome {
            content: content.to_string(),
            tool_uses: vec![],
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: Some("end_turn".into()),
        })
    }
}

#[async_trait::async_trait]
impl WorkflowLlmClient for RecordingAuthor {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.reply()
    }
    async fn run_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.calls.lock().unwrap().push((false, call.task));
        self.reply()
    }
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.calls.lock().unwrap().push((true, call.task));
        self.reply()
    }
}

fn runner_for(
    store: &WorkflowStore,
    run_id: &str,
    author: Arc<RecordingAuthor>,
    ui_sink: archon_workflow::SharedWorkflowUiSink,
) -> WorkflowV2ScriptRunner {
    let spec = test_spec();
    let client = LiveV2AgentClient::new(author, ui_sink, Vec::new(), run_id.into(), None, None);
    WorkflowV2ScriptRunner::new(
        "author stall".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2")),
        store.clone(),
        run_id.to_string(),
        true,
        None,
        None,
    )
}

const WORKLESS: &str = r#"export const meta = { name: 'workless-demo', phases: [{ title: 'Only' }] }
export default async function workflow({ phase, log }) {
  await phase("No Real Work");
  await log("nothing spawned");
  return { accepted: [], blocked: [], notes: "did nothing" };
}
"#;

/// Six rejected drafts in a row used to end the run as Failed (an error the
/// finalizer recorded as `failed`). Now the run is paused with the call, the
/// attempts and the last finding; the resume starts a new author agent with
/// that finding, numbers its attempts after the recorded ones, and the run
/// goes on to execute the script it authors.
#[tokio::test]
async fn exhausted_author_attempts_pause_and_the_resume_authors_with_a_new_agent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(test_spec()).expect("run");
    let authored_path = store.run_dir(&run.id).join("authored-workflow.js");

    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let stalled = RecordingAuthor::new(WORKLESS);
    let error = runner_for(&store, &run.id, stalled.clone(), ui_sink.clone())
        .run_authored_script_lifecycle(authored_path.clone())
        .await
        .expect_err("the stall stops this execution");
    assert!(
        matches!(&error, WorkflowError::ControlPaused(message)
            if message.contains("failed its dry-run pre-flight 6 times")
                && message.contains("plans ZERO agent calls")
                && message.contains("paused, not failed")),
        "{error:?}"
    );
    assert_eq!(stalled.calls.lock().unwrap().len(), 6);
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
    let events = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
    let pause = events
        .lines()
        .find(|line| line.contains("author_stall_pause"))
        .expect("the pause carries its evidence");
    let pause: serde_json::Value = serde_json::from_str(pause).unwrap();
    let detail = pause.get("detail").unwrap_or(&pause);
    assert_eq!(detail["call_id"], "author-workflow-script", "{detail}");
    assert_eq!(detail["stall"], "defect_attempts_exhausted");
    assert_eq!(detail["defect_attempts"], 6);
    assert_eq!(detail["last_rejection"], "rejected-scripts/attempt-6.json");
    assert!(
        detail["last_finding"]
            .as_str()
            .unwrap()
            .contains("ZERO agent calls")
    );

    // The operator resumes (as `workflow resume --live` does), and the author
    // now writes a script the pre-flight accepts.
    archon_workflow::LifecycleController::new(store.clone())
        .apply(&run.id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
    let resumed = RecordingAuthor::new(AUTHORED_DEMO_SCRIPT);
    let summary = runner_for(&store, &run.id, resumed.clone(), ui_sink)
        .run_authored_script_lifecycle(authored_path.clone())
        .await
        .expect("the resumed run goes on");
    // It authored, then executed the script to its end (NeedsReview: this
    // fixture has no task set, so its acceptance stage cannot evaluate).
    assert_eq!(summary.status, WorkflowV2Status::NeedsReview);
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    assert!(
        v2_store
            .load_call_record("phase-1-authored-phase")
            .unwrap()
            .is_some(),
        "the authored script executed"
    );
    let calls = resumed.calls.lock().unwrap();
    let (continuing, task) = &calls[0];
    assert!(!continuing, "the resume authors with a new agent");
    assert!(
        task.contains("YOUR PREVIOUS ATTEMPT WAS REJECTED") && task.contains("ZERO agent calls"),
        "the new agent is handed the last finding: {task}"
    );
    assert!(authored_path.exists(), "the accepted script is persisted");
    // The first execution's evidence is intact: nothing was renumbered over.
    let rejected = store.run_dir(&run.id).join("rejected-scripts");
    for attempt in 1..=6 {
        assert_eq!(
            std::fs::read_to_string(rejected.join(format!("attempt-{attempt}.js"))).unwrap(),
            WORKLESS.trim()
        );
    }
    assert!(!rejected.join("attempt-7.json").exists());
    assert!(
        !rejected.join("stall-pause.json").exists(),
        "the hand-over is taken once"
    );
}

/// A re-author that does not follow an author stall (an operator deleted the
/// persisted script after a run whose authoring had one rejection) starts
/// from the brief alone: an old rejection is evidence, not feedback.
#[tokio::test]
async fn a_re_author_after_no_stall_is_not_handed_an_old_rejection() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(test_spec()).expect("run");
    let detail = serde_json::json!({"attempt": 1, "error": "OLD FINDING", "script_path": null});
    store
        .write_run_json(&run.id, "rejected-scripts/attempt-1.json", &detail)
        .unwrap();
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let author = RecordingAuthor::new(AUTHORED_DEMO_SCRIPT);
    runner_for(&store, &run.id, author.clone(), ui_sink)
        .run_authored_script_lifecycle(store.run_dir(&run.id).join("authored-workflow.js"))
        .await
        .expect("the run authors and executes");
    let calls = author.calls.lock().unwrap();
    assert!(!calls[0].1.contains("OLD FINDING"), "{}", calls[0].1);
    assert!(!calls[0].1.contains("YOUR PREVIOUS ATTEMPT WAS REJECTED"));
}
