use super::*;
use archon_tools::audit_landing::{AuditLanding, LandingHost};
use serde_json::{Value,json};
#[derive(Default)]
struct Host { records: Mutex<std::collections::BTreeSet<String>> }
impl LandingHost for Host {
    fn land(&self, value:Value)->Result<String,String> {
        self.records.lock().unwrap().insert(value["declared_path"].as_str().unwrap().into()); self.hint()
    }
    fn hint(&self)->Result<String,String> { Ok(format!("Host-retained audit records: {} of 41",self.records.lock().unwrap().len())) }
    fn complete(&self,_:&Value)->Result<(),String> {
        if self.records.lock().unwrap().len()==41 {Ok(())}else{Err("missing audit paths".into())}
    }
}
#[tokio::test]
async fn audit_landing_tool_cannot_run_without_host_capability() {
    let result=archon_tools::audit_landing::LandAuditRecordTool.execute(json!({}),&ToolContext::default()).await;
    assert!(result.is_error);
}
#[tokio::test]
async fn audit_runner_repairs_incomplete_terminal_reply_without_new_session() {
    let host=Arc::new(Host::default());
    for i in 0..40 {host.land(json!({"declared_path":format!("file-{i}")})).unwrap();}
    let reply=json!({"status":"accepted","data":{"repository_audit":{"schema_version":1,"snapshot":"sealed","records_landed":41}}}).to_string();
    let provider=Arc::new(MockProvider::new(vec![text_response(&reply),
        tool_use_response("land","land-audit-record",r#"{"declared_path":"last"}"#),text_response(&reply)]));
    let mut runner=make_runner(provider.clone(),4);
    let mut registry=crate::dispatch::create_default_registry(std::env::temp_dir(),None);
    registry.replace(Box::new(archon_tools::audit_landing::LandAuditRecordTool));
    runner.tool_definitions=archon_llm::provider::shared_tools(registry.tool_definitions());
    runner.registry=Arc::new(registry);
    runner.tool_context.audit_landing=Some(Arc::new(AuditLanding::new(host.clone(),Some(30))));
    assert_eq!(runner.run("audit").await.unwrap(),reply);
    assert_eq!(host.records.lock().unwrap().len(),41);
    let requests=provider.requests();
    assert_eq!(requests.len(),3);
    assert!(serde_json::to_string(&requests[1].messages).unwrap().contains("missing audit paths"));
    assert!(serde_json::to_string(&requests[2].messages).unwrap().contains("land-audit-record"));
}
