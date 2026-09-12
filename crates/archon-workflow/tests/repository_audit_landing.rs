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
