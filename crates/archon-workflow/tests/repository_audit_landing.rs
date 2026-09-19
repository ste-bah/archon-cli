use archon_workflow::{repository_audit::{AuditContract, AuditRecord, Verdict, RequiredAction, landing::AuditLanding}, *};
use serde_json::json;
fn record(path: &str) -> AuditRecord { AuditRecord { declared_path:path.into(), verdict:Verdict::Absent,
    equivalents:vec![], required_action:RequiredAction::Deliver, reason:"Not present in sealed source".into() } }
#[test]
fn landed_records_survive_context_loss_and_reject_wrong_snapshot_or_path() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source"); std::fs::create_dir(&source).unwrap();
    let contract = AuditContract { schema_version:1, snapshot:"sealed".into(), declared_paths:(0..41).map(|i|format!("file-{i}.txt")).collect() };
    let root = temp.path().join("records/attempt-1");
    let landing = AuditLanding::open(root.clone(), source.clone(), contract.clone()).unwrap();
    for path in &contract.declared_paths[..25] { landing.land(record(path)).unwrap(); }
    drop(landing); // compaction/reconstruction has no conversation evidence to consult
    let landing = AuditLanding::open(root.clone(), source.clone(), contract.clone()).unwrap();
    assert_eq!(landing.remaining().unwrap().len(), 16);
    for path in landing.remaining().unwrap() { landing.land(record(&path)).unwrap(); }
    assert_eq!(landing.report().unwrap().records.len(), 41);
    assert!(landing.land(record("../outside")).is_err());
    let mut wrong = contract.clone(); wrong.snapshot = "different".into();
    assert!(AuditLanding::open(root, source, wrong).is_err());
}
#[tokio::test]
async fn compact_submission_is_completed_by_host_and_missing_paths_repair_locally() {
    let temp = tempfile::tempdir().unwrap();
    let contract = AuditContract { schema_version:1, snapshot:"sealed".into(), declared_paths:vec!["a".into(),"b".into()] };
    let landing = std::sync::Arc::new(AuditLanding::open(temp.path().join("records"),temp.path().into(),contract.clone()).unwrap());
    landing.land(record("a")).unwrap();
    let mut request = v2::call_data::v2_agent_request("audit",Some(temp.path().display().to_string()), &WorkflowV2CallExecution {
        call:WorkflowV2HostCall { id:"audit".into(),method:WorkflowV2HostMethod::Agent,write_mode:None,
        options:WorkflowV2HostOptions { extra:[("repository_audit_contract".into(),serde_json::to_value(contract).unwrap())].into(),..Default::default() } },
        input:json!({}),depends_on:vec![] },None);
    request.role="critic".into();
    let reply = json!({"status":"accepted",
        "data":{"repository_audit":{"schema_version":1,"snapshot":"sealed","records_landed":2}}}).to_string();
    archon_workflow::repository_audit::landing::scope(landing.clone(), async {
        let adapter = WorkflowV2AgentAdapter::new();
        let error = adapter.parse_agent_output(&request,&reply).unwrap_err().to_string();
        assert!(error.contains("b"));
        landing.land(record("b")).unwrap();
        let result = adapter.parse_agent_output(&request,&reply).unwrap();
        assert_eq!(result.data["repository_audit"]["records"].as_array().unwrap().len(),2);
    }).await;
}
fn landed(temp: &tempfile::TempDir) -> (AuditContract, AuditLanding) {
    let contract = AuditContract { schema_version:1, snapshot:"sealed".into(), declared_paths:vec!["a".into(),"b".into()] };
    let landing = AuditLanding::open(temp.path().join("records"),temp.path().into(),contract.clone()).unwrap();
    for path in &contract.declared_paths { landing.land(record(path)).unwrap(); }
    (contract, landing)
}
#[test]
fn issue49_extra_completion_keys_carry_no_authority_and_are_ignored() {
    let temp = tempfile::tempdir().unwrap();
    let (_, landing) = landed(&temp);
    let report = landing.complete(&json!({"schema_version":1,"snapshot":"sealed","records_landed":2,"remaining_paths":[],"note":"done"})).unwrap();
    assert_eq!(report.records.len(), 2);
    assert_eq!(report.snapshot, "sealed");
}
#[test]
fn issue49_wrong_count_refusal_names_both_counts_and_the_accepted_object() {
    let temp = tempfile::tempdir().unwrap();
    let (_, landing) = landed(&temp);
    let error = landing.complete(&json!({"schema_version":1,"snapshot":"sealed","records_landed":3})).unwrap_err().to_string();
    assert!(error.contains("records_landed=3 but host retained 2 record(s)"), "{error}");
    assert!(error.contains(r#"{"schema_version":1,"snapshot":"sealed","records_landed":2}"#), "{error}");
    let missing = landing.complete(&json!({"schema_version":1,"snapshot":"sealed"})).unwrap_err().to_string();
    assert!(missing.contains("records_landed=missing but host retained 2 record(s)"), "{missing}");
}
#[test]
fn issue49_wrong_snapshot_or_schema_refusal_names_the_field() {
    let temp = tempfile::tempdir().unwrap();
    let (_, landing) = landed(&temp);
    let error = landing.complete(&json!({"schema_version":1,"snapshot":"other","records_landed":2})).unwrap_err().to_string();
    assert!(error.contains(r#"snapshot="other" but host snapshot is "sealed""#), "{error}");
    assert!(error.contains(r#"{"schema_version":1,"snapshot":"sealed","records_landed":2}"#), "{error}");
    let error = landing.complete(&json!({"schema_version":2,"snapshot":"sealed","records_landed":2})).unwrap_err().to_string();
    assert!(error.contains("schema_version=2 but host requires 1"), "{error}");
    let error = landing.complete(&json!(["not","an","object"])).unwrap_err().to_string();
    assert!(error.contains("must be a JSON object"), "{error}");
}
#[test]
fn issue49_hint_ends_with_the_exact_accepted_completion_object() {
    let temp = tempfile::tempdir().unwrap();
    let (_, landing) = landed(&temp);
    let hint = landing.hint().unwrap();
    assert!(hint.ends_with(r#"{"schema_version":1,"snapshot":"sealed","records_landed":2}"#), "{hint}");
    assert!(!hint.contains("records_landed=2"), "{hint}");
}
#[tokio::test]
async fn issue49_v2_adapter_accepts_compact_completion_with_echoed_remaining_paths() {
    let temp = tempfile::tempdir().unwrap();
    let (contract, landing) = landed(&temp);
    let landing = std::sync::Arc::new(landing);
    let mut request = v2::call_data::v2_agent_request("audit",Some(temp.path().display().to_string()), &WorkflowV2CallExecution {
        call:WorkflowV2HostCall { id:"audit".into(),method:WorkflowV2HostMethod::Agent,write_mode:None,
        options:WorkflowV2HostOptions { extra:[("repository_audit_contract".into(),serde_json::to_value(contract).unwrap())].into(),..Default::default() } },
        input:json!({}),depends_on:vec![] },None);
    request.role="critic".into();
    let reply = json!({"status":"accepted",
        "data":{"repository_audit":{"schema_version":1,"snapshot":"sealed","records_landed":2,"remaining_paths":[]}}}).to_string();
    archon_workflow::repository_audit::landing::scope(landing.clone(), async {
        let result = WorkflowV2AgentAdapter::new().parse_agent_output(&request,&reply).unwrap();
        assert_eq!(result.data["repository_audit"]["records"].as_array().unwrap().len(),2);
    }).await;
}
#[test]
fn issue51_hint_names_the_carried_paths_and_lists_only_the_delta_as_remaining() {
    let temp = tempfile::tempdir().unwrap();
    let contract = AuditContract { schema_version:1, snapshot:"sealed".into(), declared_paths:vec!["a".into()] };
    let landing = AuditLanding::open(temp.path().join("delta"),temp.path().into(),contract).unwrap().carrying(706);
    let hint = landing.hint().unwrap();
    assert!(hint.contains("0 of 1 landed"), "{hint}");
    assert!(hint.contains(r#"Remaining paths: ["a"]."#), "{hint}");
    assert!(hint.contains("706 other declared path(s) keep their prior verdict"), "{hint}");
    assert!(hint.ends_with(r#"{"schema_version":1,"snapshot":"sealed","records_landed":1}"#), "{hint}");
    let (_, plain) = landed(&temp);
    assert!(!plain.hint().unwrap().contains("keep their prior verdict"), "nothing carried, nothing said");
}
