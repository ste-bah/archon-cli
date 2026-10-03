//! Issue-245: log redaction must never touch authoritative run state, and the
//! public records must still be redacted.

use archon_workflow::events::{REDACTION_MARKER, redaction_marker_path};
use archon_workflow::{
    WorkflowEventKind, WorkflowEventLog, WorkflowSpec, WorkflowStore, WorkflowV2BranchOutcome,
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status,
};
use serde_json::json;

const AUTHORED: &str =
    "refuse() {\n  lane=\"$1\"\n  token=\"$3\"\n  api_key=None\n  password=x\n  secret='s'\n}\n";

fn agent_record(content: &str) -> WorkflowV2CallRecord {
    let call = WorkflowV2HostCall {
        id: "acceptance-author-AC-X-001-1".to_string(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: WorkflowV2HostOptions::default(),
    };
    let mut result = WorkflowV2Result::accepted("trusted raw provider outcome captured");
    result.data = json!({
        "content": content,
        "stopReason": "end_turn",
        "reasoning": "a data key a log record would strip",
    });
    WorkflowV2CallRecord::new("wf-test", call, 1, "hash".into(), result, Vec::new())
}

#[test]
fn a_call_record_is_stored_exactly_as_produced() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    store.save_call_record(&agent_record(AUTHORED)).unwrap();

    let path = store.result_path("acceptance-author-AC-X-001-1");
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(!raw.contains(REDACTION_MARKER), "{raw}");
    let loaded = store
        .load_call_record("acceptance-author-AC-X-001-1")
        .unwrap()
        .expect("stored record");
    assert_eq!(loaded.result.data["content"].as_str(), Some(AUTHORED));
    assert_eq!(
        loaded.result.data["reasoning"].as_str(),
        Some("a data key a log record would strip")
    );
    assert_eq!(
        redaction_marker_path(&serde_json::to_value(&loaded).unwrap()),
        None
    );
}

#[test]
fn a_branch_outcome_is_stored_exactly_as_produced() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let mut result = WorkflowV2Result::accepted(AUTHORED);
    result.data = json!({"api_key_env": "PROVIDER_KEY", "note": AUTHORED});
    let outcome = WorkflowV2BranchOutcome {
        item_id: "item-a".into(),
        role: "coder".into(),
        status: WorkflowV2Status::Accepted,
        result: Some(result),
        error: None,
        failure_kind: None,
        item_input_hash: Some("hash-a".into()),
        completion_evidence: Vec::new(),
    };
    store.save_branch_outcome("wave-1", &outcome).unwrap();

    let loaded = store
        .load_branch_outcome("wave-1", "item-a")
        .unwrap()
        .expect("stored outcome");
    assert_eq!(loaded, outcome);
}

fn spec() -> WorkflowSpec {
    WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: redaction-test
task: Redaction test
stages:
  - id: a
    kind: agent
    agent: tester
"#,
    )
    .unwrap()
}

#[test]
fn public_event_records_still_redact_real_looking_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    let run = store.create_run(spec()).unwrap();
    let anthropic = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
    WorkflowEventLog::new(store.clone())
        .emit(
            &run.id,
            1,
            WorkflowEventKind::StageStarted,
            json!({
                "summary": format!("key {anthropic} used"),
                "header": "Authorization: Bearer opaque-bearer-credential-123",
            }),
        )
        .unwrap();

    let raw = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
    assert!(!raw.contains("sk-ant-"), "{raw}");
    assert!(!raw.contains("opaque-bearer-credential-123"), "{raw}");
    assert!(raw.contains(REDACTION_MARKER), "{raw}");
}

#[test]
fn the_marker_finder_reports_only_the_redactor_shape() {
    let carried = json!({"entries": [{"id": "A", "check": {"command": "x\n  <redacted>  if y"}}]});
    assert_eq!(
        redaction_marker_path(&carried).as_deref(),
        Some("/entries/0/check/command")
    );
    let quoted = json!({"command": "grep -q '<redacted>' out.log && echo x<redacted>"});
    assert_eq!(redaction_marker_path(&quoted), None);
}
