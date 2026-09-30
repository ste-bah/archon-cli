//! Issue-213 C5: a call whose host process died leaves a record at next start.

use super::*;

/// Never asked: the orphan is recorded before the script makes any call.
struct UnusedLlm;

#[async_trait::async_trait]
impl archon_workflow::WorkflowLlmClient for UnusedLlm {
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
    }
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

    let (ui_sink, _rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        std::sync::Arc::new(UnusedLlm),
        ui_sink,
        Vec::new(),
        run.id.clone(),
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
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    );
    let _ = runner.run(NO_CALL_SCRIPT).await;

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

#[test]
fn a_marker_of_a_live_host_is_not_an_orphan() {
    assert!(!host_alive(std::process::id()), "this process never counts");
    assert!(!host_alive(0));
    #[cfg(unix)]
    assert!(host_alive(1), "pid 1 is always alive");
}
