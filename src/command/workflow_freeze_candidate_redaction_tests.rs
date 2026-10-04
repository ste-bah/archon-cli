//! Issue-245: an author's verifier text reaches the stored result and the
//! freeze-acceptance input byte-identical, read back from disk; a candidate
//! carrying the redaction marker is refused naming the check and field.

use archon_workflow::{
    HostCommandRequest, WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2HostOptions, WorkflowV2Result, WorkflowV2ResultStore, host_command_call_id,
};
use serde_json::json;
use std::collections::BTreeMap;

const COMMAND: &str = "set -eu\nrefuse() {\n  lane=\"$1\"\n  token=\"$3\"\n  api_key=None\n  password=x\n  if run \"$lane\"; then exit 1; fi\n}\nrefuse a b c\n";

fn record(
    id: &str,
    method: WorkflowV2HostMethod,
    options: WorkflowV2HostOptions,
    data: serde_json::Value,
) -> WorkflowV2CallRecord {
    let call = WorkflowV2HostCall {
        id: id.into(),
        method,
        write_mode: None,
        options,
    };
    let mut result = WorkflowV2Result::accepted("captured");
    result.data = data;
    WorkflowV2CallRecord::new("wf-test", call, 1, "hash".into(), result, Vec::new())
}

fn call_id(stdin: &str) -> String {
    host_command_call_id(
        "freeze-acceptance",
        "catalog",
        "rev",
        &BTreeMap::new(),
        stdin.as_bytes(),
    )
}

#[test]
fn authored_verifier_text_reaches_the_freeze_input_byte_identical() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let entry = json!({"id":"AC-X-001","criterion":"","check":{"kind":"command","command":COMMAND,"cwd":"project_root"},"gap_permitted":false});
    // The author's raw reply, as the trusted raw-outcome path stores it.
    let content = serde_json::to_string(&entry).unwrap();
    let author = record(
        "acceptance-author-AC-X-001-1",
        WorkflowV2HostMethod::Agent,
        WorkflowV2HostOptions::default(),
        json!({"content": content, "stopReason": "end_turn"}),
    );
    store.save_call_record(&author).unwrap();
    let replayed = store
        .load_call_record("acceptance-author-AC-X-001-1")
        .unwrap()
        .unwrap();
    let replayed_content = replayed.result.data["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(replayed_content, content);

    // The script assembles the freeze stdin from the reply it was handed.
    let parsed: serde_json::Value = serde_json::from_str(&replayed_content).unwrap();
    let stdin = serde_json::to_string(&json!({"entries": [parsed]})).unwrap();
    let options = WorkflowV2HostOptions {
        host_command: Some(
            HostCommandRequest::new("freeze-acceptance", Some(stdin.clone())).unwrap(),
        ),
        ..WorkflowV2HostOptions::default()
    };
    let id = call_id(&stdin);
    store
        .save_call_record(&record(
            &id,
            WorkflowV2HostMethod::HostCommand,
            options,
            json!({}),
        ))
        .unwrap();
    let frozen = store.load_call_record(&id).unwrap().unwrap();
    let stored_stdin = frozen.call.options.host_command.unwrap().stdin.unwrap();
    assert_eq!(stored_stdin, stdin);
    assert_eq!(
        call_id(&stored_stdin),
        id,
        "the stored stdin is the stdin that ran"
    );

    let bytes = super::acceptance_candidate(stored_stdin.as_bytes()).expect("candidate accepted");
    let contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&bytes).unwrap();
    match &contract.acceptance[0].check {
        archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } => {
            assert_eq!(command, COMMAND)
        }
        other => panic!("unexpected check {other:?}"),
    }
}

#[test]
fn a_candidate_carrying_the_redaction_marker_is_refused_naming_check_and_field() {
    let redacted = COMMAND.replace("token=\"$3\"\n", "<redacted>  ");
    let candidate = json!({"entries":[
        {"id":"AC-X-001","criterion":"","check":{"kind":"command","command":"test -f x","cwd":"project_root"},"gap_permitted":false},
        {"id":"AC-X-002","criterion":"","check":{"kind":"command","command":redacted,"cwd":"project_root"},"gap_permitted":false}
    ]});
    let error = super::acceptance_candidate(&serde_json::to_vec(&candidate).unwrap())
        .expect_err("a redacted check is never frozen")
        .to_string();
    assert!(
        error.starts_with("check 'AC-X-002': field '/entries/1/check/command' contains the log-redaction marker `<redacted>` as a standalone word; restore the original value"),
        "{error}"
    );
    assert!(
        error.contains("quote it or build it (e.g. '<'+'redacted>')"),
        "{error}"
    );
    assert!(!error.contains("AC-X-001"), "{error}");
}

#[test]
fn prose_and_host_owned_fields_never_refuse() {
    let candidate = json!({"entries":[{
        "id":"AC-X-001",
        "criterion":"secrets print as <redacted> in logs",
        "check":{"kind":"command","command":"grep -q '<redacted>' out.log","cwd":"project_root"},
        "gap_permitted":false,
        "judgment":{"verdict":"","counterexample":"","reason":"the log shows <redacted> here","host_call_id":""}
    }]});
    let bytes = super::acceptance_candidate(&serde_json::to_vec(&candidate).unwrap())
        .expect("prose mentions and a quoted marker in a grep are accepted");
    let contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&bytes).unwrap();
    match &contract.acceptance[0].check {
        archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } => {
            assert_eq!(command, "grep -q '<redacted>' out.log")
        }
        other => panic!("unexpected check {other:?}"),
    }
}

#[test]
fn a_skeleton_marker_is_refused_only_in_staged_fields() {
    let mut skeleton = json!({"schema_version":1,"acceptance_digest":"<redacted>","tasks":[{
        "task_id":"TASK-A-001","file_name":"TASK-A-001.md","depends_on":[],"blocks":[],"implements":[],
        "deliverable_contracts":[{"kind":"file","artifact_path":"out/a.json"}]
    }]});
    assert_eq!(super::skeleton_marker_refusal(&skeleton), None);
    skeleton["tasks"][0]["deliverable_contracts"][0]["artifact_path"] = json!("out/ <redacted>");
    let reason = super::skeleton_marker_refusal(&skeleton).expect("refused");
    assert!(
        reason.starts_with(
            "task 'TASK-A-001': field '/tasks/0/deliverable_contracts/0/artifact_path' contains"
        ),
        "{reason}"
    );
}
