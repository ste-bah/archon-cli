//! Issue-213 C5: a call whose host process died leaves a record at next start.

use super::*;

/// Never asked: the orphan is recorded before the script makes any call.
struct UnusedLlm;

#[async_trait::async_trait]
impl archon_workflow::WorkflowLlmClient for UnusedLlm {
    /// Scripted replies stand for one continued session (#241).
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.run_agent(call).await
    }

    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        panic!("the orphan test script makes no agent call");
    }
}

const NO_CALL_SCRIPT: &str = r#"
export const meta = { name: 'orphan-demo', phases: [{ title: 'Only' }] }

phase('Only')
return { ok: true }
"#;

fn spec() -> archon_workflow::WorkflowSpec {
    archon_workflow::WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "orphaned-call-test".to_string(),
        task: "test".to_string(),
        target_repository_root: None,
        max_parallelism: 4,
        max_agents: 16,
        stages: Vec::new(),
        permissions: std::collections::BTreeMap::new(),
        learning_hooks: Vec::new(),
    }
}

fn marker(call_id: &str) -> InflightMarker {
    InflightMarker {
        call: WorkflowV2HostCall {
            id: call_id.to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        attempt: 2,
        input_hash: "in-hash".to_string(),
        started_at: "2026-09-30T00:00:00+00:00".to_string(),
        depends_on: Vec::new(),
        host_pid: std::process::id(),
        agent_sessions: Vec::new(),
        progress: None,
    }
}

/// Start a run on `workflow_store`, which records its orphans first.
async fn start_run(workflow_store: WorkflowStore, v2_store: WorkflowV2ResultStore, run_id: &str) {
    let (ui_sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        std::sync::Arc::new(UnusedLlm),
        ui_sink,
        Vec::new(),
        run_id.to_string(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "orphaned call".to_string(),
        WorkflowV2ScriptRuntime {
            target_repository_root: None,
            generated_config: archon_core::config::GeneratedWorkflowConfig::default(),
        },
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store,
        workflow_store,
        run_id.to_string(),
        true,
        None,
        None,
    );
    let _ = runner.run(NO_CALL_SCRIPT).await;
}

#[tokio::test]
async fn a_call_its_host_died_under_is_recorded_at_the_next_start() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec()).expect("run");
    let v2_root = workflow_store.run_dir(&run.id).join("v2");
    let v2_store = WorkflowV2ResultStore::new(v2_root.clone());
    let inflight = v2_root.join("inflight");
    std::fs::create_dir_all(&inflight).expect("inflight dir");
    for id in ["agent-7", "agent-8"] {
        std::fs::write(
            inflight.join(marker_name(id)),
            serde_json::to_vec(&marker(id)).expect("marker json"),
        )
        .expect("marker");
    }
    // agent-8 already has an answer: it is kept, never replaced by the note.
    let kept = WorkflowV2CallRecord::new(
        run.id.clone(),
        marker("agent-8").call,
        1,
        "old".to_string(),
        WorkflowV2Result {
            status: WorkflowV2Status::Accepted,
            summary: "earlier answer".to_string(),
            ..WorkflowV2Result::default()
        },
        Vec::new(),
    );
    v2_store.save_call_record(&kept).expect("seed");

    start_run(workflow_store, v2_store.clone(), &run.id).await;

    let record = v2_store
        .load_call_record("agent-7")
        .expect("lookup")
        .expect("the orphaned call must leave a record");
    assert_eq!(record.status, WorkflowV2Status::NeedsReview);
    assert_eq!(record.attempt, 2);
    assert_eq!(record.result.data["interrupted"], ORPHANED_REASON);
    assert_eq!(record.result.data["host_pid"], std::process::id());
    assert!(!record.is_reusable_for(&record.input_hash));

    let untouched = v2_store
        .load_call_record("agent-8")
        .expect("lookup")
        .expect("kept");
    assert_eq!(untouched.status, WorkflowV2Status::Accepted);
    assert_eq!(untouched.result.summary, "earlier answer");

    assert_eq!(
        std::fs::read_dir(&inflight).expect("dir").count(),
        0,
        "every marker is consumed"
    );
}

/// Issue-213 C5: a host killed mid-call leaves the marker its refresh last
/// wrote, and the next start's record of the call carries what its sessions
/// were doing — turns, last tool call, touched paths — and which sessions
/// they were. The kill is simulated as above: the marker is left on disk and
/// no record is written for the call.
#[tokio::test]
async fn a_killed_call_is_recorded_with_its_sessions_progress() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec()).expect("run");
    let v2_root = workflow_store.run_dir(&run.id).join("v2");
    let v2_store = WorkflowV2ResultStore::new(v2_root.clone());
    let call_id = "implement-killed-3";
    // What the live client and the runner note while the call runs.
    let session = format!("{}-{call_id}-attempt-1", run.id);
    let agent = format!("{session}-0-coder-u");
    crate::command::workflow_live::workflow_live_v2::workflow_live_v2_client::call_sessions::note_session(
        &run.id, call_id, &session,
    );
    archon_tools::session_progress::note_turn(&agent, 41);
    archon_tools::session_progress::note_tool_call(
        &agent,
        "Bash",
        &serde_json::json!({"command": "make check"}),
    );
    archon_tools::session_progress::note_touched(&agent, std::path::Path::new("/w/src/a.rs"));
    // The marker as the host's refresh writes it, then the host dies.
    let execution = WorkflowV2CallExecution {
        call: marker(call_id).call,
        input: serde_json::json!({}),
        depends_on: Vec::new(),
    };
    let refreshed = InflightMarker::now(&run.id, &execution, 1, "in-hash");
    write_marker(&v2_root.join("inflight"), &refreshed).expect("marker");

    start_run(workflow_store, v2_store.clone(), &run.id).await;

    let record = v2_store
        .load_call_record(call_id)
        .expect("lookup")
        .expect("the killed call must leave a record");
    assert_eq!(record.status, WorkflowV2Status::NeedsReview);
    assert_eq!(record.result.data["interrupted"], ORPHANED_REASON);
    assert_eq!(record.result.data["turns"], 41);
    assert!(
        record.result.data["last_tool_call"]
            .as_str()
            .is_some_and(|call| call.starts_with("Bash") && call.contains("make check")),
        "{}",
        record.result.data
    );
    assert_eq!(
        record.result.data["touched_paths"],
        serde_json::json!(["/w/src/a.rs"])
    );
    assert_eq!(record.agent_session_id.as_deref(), Some(session.as_str()));
}

#[test]
fn interruption_progress_is_empty_without_sessions() {
    let progress = super::super::workflow_live_v2_script_host_interrupt::interruption_progress(&[]);
    assert_eq!(progress, serde_json::json!({}));
}

#[test]
fn interruption_progress_names_turns_last_call_and_touched_paths() {
    let session = "wf-inflight-test-stage-agent-9-attempt-1";
    let agent = format!("{session}-0-coder-u");
    archon_tools::session_progress::note_turn(&agent, 12);
    archon_tools::session_progress::note_tool_call(
        &agent,
        "Edit",
        &serde_json::json!({"file_path": "a.rs"}),
    );
    archon_tools::session_progress::note_touched(&agent, std::path::Path::new("/w/a.rs"));
    let progress = super::super::workflow_live_v2_script_host_interrupt::interruption_progress(&[
        session.to_string(),
    ]);
    assert_eq!(progress["turns"], 12);
    assert!(
        progress["last_tool_call"]
            .as_str()
            .is_some_and(|call| call.starts_with("Edit")),
        "{progress}"
    );
    assert_eq!(progress["touched_paths"], serde_json::json!(["/w/a.rs"]));
    assert_eq!(progress["agent_progress"][0]["agent_id"], agent);
}

/// Issue 339: on every platform, a running other process owns its marker, and
/// an ended one, this process and pid 0 do not.
#[test]
fn a_marker_of_a_live_host_is_not_an_orphan() {
    assert!(!host_alive(std::process::id()), "this process never counts");
    assert!(!host_alive(0));
    let mut host = archon_test_support::live_process::LiveChild::spawn();
    let pid = host.pid();
    assert!(host_alive(pid), "a running host (pid {pid}) is alive");
    host.end();
    assert!(!host_alive(pid), "an ended host (pid {pid}) is not");
}

/// Issue-213 C5 (review): the refresh, for real. Under a paused clock the
/// marker written before dispatch carries no progress; once the call's session
/// reports some and the refresh interval passes, the marker on disk carries it,
/// and still the original dispatch time.
#[tokio::test(start_paused = true)]
async fn the_marker_is_refreshed_with_new_progress_and_keeps_its_dispatch_time() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dir = temp.path().join("inflight");
    let run = "wf-refresh-test";
    let call_id = "implement-refresh-1";
    let execution = WorkflowV2CallExecution {
        call: marker(call_id).call,
        input: serde_json::json!({}),
        depends_on: Vec::new(),
    };
    let read = || {
        serde_json::from_slice::<InflightMarker>(
            &std::fs::read(dir.join(marker_name(call_id))).expect("marker on disk"),
        )
        .expect("marker json")
    };
    let (done, finished) = tokio::sync::oneshot::channel::<()>();
    let store = WorkflowV2ResultStore::new(dir.parent().unwrap().join("v2"));
    let work = refresh_while(&store, &dir, run, &execution, 1, "in-hash", finished);
    tokio::pin!(work);
    let zero = std::time::Duration::ZERO;
    assert!(
        tokio::time::timeout(zero, &mut work).await.is_err(),
        "still running"
    );
    let first = read();
    assert!(first.progress.is_none() && first.agent_sessions.is_empty());

    let session = format!("{run}-{call_id}-attempt-1");
    crate::command::workflow_live::workflow_live_v2::workflow_live_v2_client::call_sessions::note_session(
        run, call_id, &session,
    );
    archon_tools::session_progress::note_turn(&format!("{session}-0-coder-u"), 9);
    tokio::time::advance(INFLIGHT_REFRESH).await;
    assert!(
        tokio::time::timeout(zero, &mut work).await.is_err(),
        "still running"
    );

    let refreshed = read();
    assert_eq!(refreshed.agent_sessions, vec![session]);
    assert_eq!(refreshed.progress.as_ref().expect("progress")["turns"], 9);
    assert_eq!(
        refreshed.started_at, first.started_at,
        "the dispatch time is kept"
    );
    done.send(()).expect("send");
    work.await.expect("the call returns");
}

/// Issue 303: a `Running` record with no marker never dispatched; the next
/// start closes it. One whose marker a live host still holds is left alone.
#[tokio::test]
async fn a_running_record_without_a_marker_is_closed_at_the_next_start() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec()).expect("run");
    let v2_root = workflow_store.run_dir(&run.id).join("v2");
    let v2_store = WorkflowV2ResultStore::new(v2_root.clone());
    let running = |id: &str| {
        WorkflowV2CallRecord::new(
            run.id.clone(),
            marker(id).call,
            3,
            "in-hash".to_string(),
            WorkflowV2Result {
                status: WorkflowV2Status::Running,
                summary: "fixed decomposition call in flight".to_string(),
                ..WorkflowV2Result::default()
            },
            Vec::new(),
        )
    };
    let mut state = run.clone();
    for id in ["unstarted", "live"] {
        v2_store.save_call_record(&running(id)).expect("seed");
        let mut stage = archon_workflow::run::StageState::pending(id);
        stage.status = archon_workflow::StageStatus::Running;
        state.stages.insert(id.to_string(), stage);
    }
    workflow_store.save_state(&state).expect("running stages");
    // Issue 339: a running child is another live host on every platform (pid
    // 1 is no process on Windows); its call is its own.
    let host = archon_test_support::live_process::LiveChild::spawn();
    let mut live = marker("live");
    live.host_pid = host.pid();
    let inflight = v2_root.join("inflight");
    std::fs::create_dir_all(&inflight).expect("inflight dir");
    std::fs::write(
        inflight.join(marker_name("live")),
        serde_json::to_vec(&live).expect("marker json"),
    )
    .expect("marker");

    start_run(workflow_store.clone(), v2_store.clone(), &run.id).await;

    let stages = workflow_store.load_state(&run.id).expect("state").stages;
    assert_eq!(
        stages["unstarted"].status,
        archon_workflow::StageStatus::NeedsReview
    );
    assert_eq!(stages["live"].status, archon_workflow::StageStatus::Running);
    let closed = v2_store
        .load_call_record("unstarted")
        .expect("lookup")
        .expect("record");
    assert_eq!(closed.status, WorkflowV2Status::NeedsReview);
    assert_eq!(closed.attempt, 3);
    assert_eq!(closed.result.data["interrupted"], "dispatch_not_started");
    assert_eq!(closed.result.data["inflight_marker"], false);
    assert!(!closed.is_reusable_for(&closed.input_hash));
    let kept = v2_store
        .load_call_record("live")
        .expect("lookup")
        .expect("record");
    assert_eq!(
        kept.status,
        WorkflowV2Status::Running,
        "a live host owns it"
    );
    drop(host);
}
