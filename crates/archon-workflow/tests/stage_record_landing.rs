use archon_workflow::{v2::record_landing::{RecordLanding, RecordKind}, *};
use serde_json::json;
#[test]
fn review_records_survive_reopen_and_require_every_subject_even_with_no_findings() {
    let temp=tempfile::tempdir().unwrap();
    let path=temp.path().join("records");
    let landing=RecordLanding::open(path.clone(),"identity".into(),RecordKind::Review,vec!["one".into(),"two".into()]).unwrap();
    landing.land(json!({"subject":"one","findings":[{"id":"F1","claim":"counterexample"}],"evidence":[{"kind":"inspection","summary":"read implementation"}]})).unwrap();
    drop(landing);
    let landing=RecordLanding::open(path.clone(),"identity".into(),RecordKind::Review,vec!["one".into(),"two".into()]).unwrap();
    assert_eq!(landing.remaining().unwrap(),vec!["two"]);
    assert!(landing.assemble(&json!({"records_landed":2})).is_err());
    landing.land(json!({"subject":"two","findings":[],"evidence":[{"kind":"inspection","summary":"checked boundary"}]})).unwrap();
    let data=landing.assemble(&json!({"records_landed":2})).unwrap();
    assert_eq!(data["findings"][0]["id"],"F1");
    assert!(RecordLanding::open(path,"other identity".into(),RecordKind::Review,vec!["one".into()]).is_err());
}
#[test]
fn verification_records_preserve_failures_and_commands() {
    let temp=tempfile::tempdir().unwrap();
    let landing=RecordLanding::open(temp.path().into(),"identity".into(),RecordKind::Verify,vec!["unit".into()]).unwrap();
    assert!(landing.land(json!({"subject":"unit","status":"accepted","findings":[],"evidence":[]})).is_err());
    landing.land(json!({"subject":"unit","status":"failed","summary":"test failed","findings":[{"claim":"wrong output"}],
        "commands_run":[{"kind":"test","command":"test-unit","status":"failed","exit_code":1,"output_summary":"assertion failed"}],
        "evidence":[{"kind":"test","summary":"assertion failed"}]})).unwrap();
    let data=landing.assemble(&json!({"records_landed":1})).unwrap();
    assert_eq!(data["verification_records"][0]["status"],"failed");
}
