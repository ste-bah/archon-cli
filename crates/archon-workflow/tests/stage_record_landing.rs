use archon_workflow::{v2::record_landing::{RecordLanding, RecordKind}, *};
use serde_json::json;
#[test]
fn review_records_survive_reopen_and_require_every_subject_even_with_no_findings() {
    let temp=tempfile::tempdir().unwrap();
    let path=temp.path().join("records");
    let landing=RecordLanding::open(path.clone(),"identity".into(),RecordKind::Review,vec!["one".into(),"two".into()],true).unwrap();
    landing.land(json!({"subject":"one","findings":[{"id":"F1","claim":"counterexample"}],"evidence":[{"kind":"inspection","summary":"read implementation"}]})).unwrap();
    drop(landing);
    let landing=RecordLanding::open(path.clone(),"identity".into(),RecordKind::Review,vec!["one".into(),"two".into()],true).unwrap();
    assert_eq!(landing.remaining().unwrap(),vec!["two"]);
    assert!(landing.assemble(&json!({"records_landed":2})).is_err());
    landing.land(json!({"subject":"two","findings":[],"evidence":[{"kind":"inspection","summary":"checked boundary"}]})).unwrap();
    let data=landing.assemble(&json!({"records_landed":2})).unwrap();
    assert_eq!(data["findings"][0]["id"],"F1");
    assert!(RecordLanding::open(path,"other identity".into(),RecordKind::Review,vec!["one".into()],true).is_err());
}
#[test]
fn verification_records_preserve_failures_and_commands() {
    let temp=tempfile::tempdir().unwrap();
    let landing=RecordLanding::open(temp.path().into(),"identity".into(),RecordKind::Verify,vec!["unit".into()],true).unwrap();
    assert!(landing.land(json!({"subject":"unit","status":"accepted","findings":[],"evidence":[]})).is_err());
    landing.land(json!({"subject":"unit","status":"failed","summary":"test failed","findings":[{"claim":"wrong output"}],
        "commands_run":[{"kind":"test","command":"test-unit","status":"failed","exit_code":1,"output_summary":"assertion failed"}],
        "evidence":[{"kind":"test","summary":"assertion failed"}]})).unwrap();
    let data=landing.assemble(&json!({"records_landed":1})).unwrap();
    assert_eq!(data["verification_records"][0]["status"],"failed");
}

#[test]
fn multiple_landings_for_subject_preserve_earlier_findings() {
    let temp=tempfile::tempdir().unwrap();
    let records=RecordLanding::open(temp.path().into(),"id".into(),RecordKind::Review,vec!["unit".into()],true).unwrap();
    for id in ["F1","F2"] {
        records.land(json!({"subject":"unit","findings":[{"id":id,"claim":id}],"evidence":[{"kind":"inspection","summary":id}]})).unwrap();
    }
    assert_eq!(records.assemble(&json!({"records_landed":1})).unwrap()["findings"].as_array().unwrap().len(),2);
}
#[test]
fn skeleton_landing_retains_typed_entries_and_rejects_invalid_filenames() {
    let temp=tempfile::tempdir().unwrap();
    let records=RecordLanding::open(temp.path().into(),"id".into(),RecordKind::Skeleton,vec![],false).unwrap();
    assert!(records.land(json!({"subject":"TASK-X-001","task":{"task_id":"TASK-X-001","file_name":"../escape.md"}})).is_err());
    records.land(json!({"subject":"TASK-X-001","task":{"task_id":"TASK-X-001","file_name":"TASK-X-001.md"}})).unwrap();
    let data=records.assemble(&json!({"records_landed":1})).unwrap();
    assert_eq!(data["tasks"][0]["task_id"],"TASK-X-001");
}
