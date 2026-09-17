//! Host bridge for typed single-artifact stage evidence.
use archon_workflow::{v2::record_landing::{RecordLanding,RecordKind},*};
use std::sync::Arc;
use serde_json::{Value,json};

pub(super) fn prepare(request:&mut WorkflowV2AgentRequest,store:Option<&WorkflowV2ResultStore>)
    -> WorkflowResult<Option<Arc<RecordLanding>>> {
    let Some(store)=store else{return Ok(None)};
    if request.is_write_capable() || request.call.options.extra.contains_key("repository_audit_contract") {return Ok(None);}
    let extra=&request.call.options.extra;
    let kind=if extra.get("recordLanding").and_then(Value::as_str)==Some("skeleton") {RecordKind::Skeleton}
        else if extra.contains_key("reviewContract") || extra.contains_key("review_contract") {RecordKind::Review}
        else if request.call.options.item_kind.as_deref()==Some("focused_verification")
            || request.input.get("item_kind").and_then(Value::as_str)==Some("focused_verification")
            || request.input.get("focused_verification").is_some()
            || request.input.pointer("/item/focused_verification").is_some() {RecordKind::Verify}
        else {return Ok(None)};
    let mut subjects=archon_workflow::v2::branch_stamping::branch_canonical_task_ids(&request.input);
    // Issue-37: a reduce call has no canonical task ids, so its subject is the call id and must not be stamped onto findings as their task.
    let subjects_are_tasks=kind!=RecordKind::Skeleton && !subjects.is_empty();
    if kind==RecordKind::Skeleton {subjects.clear();}
    else if subjects.is_empty(){subjects.push(request.call.id.clone());}
    subjects.sort();subjects.dedup();
    use sha2::{Digest,Sha256};
    let identity=format!("{:x}",Sha256::digest(serde_json::to_vec(request)?));
    let records=Arc::new(RecordLanding::open(store.root().join("stage-records").join(&identity).join(uuid::Uuid::new_v4().to_string()),identity,kind,subjects,subjects_are_tasks)?);
    request.task.push_str(&format!("\n{}",records.hint()?));
    Ok(Some(records))
}
struct Bridge(Arc<RecordLanding>);
impl archon_tools::audit_landing::LandingHost for Bridge {
    fn tool_name(&self)->&'static str {self.0.tool_name()}
    fn schema(&self)->Option<Value>{Some(json!({"type":"object","required":["subject"],"additionalProperties":false,"description":archon_workflow::v2::record_landing::schema_hint(),"properties":{
        "subject":{"type":"string"},"findings":{"type":"array","items":{"type":"object"}},"evidence":{"type":"array","items":{"type":"object"}},
        "commands_run":{"type":"array","items":{"type":"object"}},"status":{"type":"string"},"summary":{"type":"string"},
        // Issue-38: `FrozenTask` deserialises with serde defaults and accepts unknown keys, so `task` stays open; the listed properties are the fields the host reads.
        "task":{"type":"object","description":"Skeleton records only: one full skeleton entry","required":["task_id","file_name"],"properties":{
            "task_id":{"type":"string"},"file_name":{"type":"string"},
            "depends_on":{"type":"array","items":{"type":"object","required":["task_id"],"properties":{"task_id":{"type":"string"},"consumes":{"type":"array","items":{"type":"object","required":["artifact_path"],"properties":{"artifact_path":{"type":"string"}}}},"ordering_only":{"type":"boolean"}}}},
            "blocks":{"type":"array","items":{"type":"string"}},"implements":{"type":"array","items":{"type":"string"}},
            "deliverable_contracts":{"type":"array","items":{"type":"object","required":["kind","artifact_path"],"properties":{"kind":{"type":"string"},"artifact_path":{"type":"string"},"min_instances":{"type":"integer"}}}}}},
        "replace":{"type":"boolean","description":"Supersede the earlier record for this subject instead of unioning with it"}}}))}
    fn land(&self,value:Value)->Result<String,String>{self.0.land(value).map_err(|e|e.to_string())?;self.hint()}
    fn hint(&self)->Result<String,String>{self.0.hint().map_err(|e|e.to_string())}
    fn complete(&self,value:&Value)->Result<(),String>{self.0.assemble(value).map(|_|()).map_err(|e|e.to_string())}
}
pub(super) async fn scope<T>(records:Option<Arc<RecordLanding>>,work:impl std::future::Future<Output=T>)->T {
    if let Some(records)=records {
        let capability=Arc::new(archon_tools::audit_landing::AuditLanding::new(Arc::new(Bridge(records.clone())),None));
        archon_workflow::v2::record_landing::scope(records,archon_tools::audit_landing::scope(capability,work)).await
    }else{work.await}
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::*;
    struct Port;
    #[async_trait::async_trait]
    impl WorkflowLlmClient for Port {
        async fn send_message(&self,_:Vec<Value>,_:Vec<Value>,_:Vec<Value>,_:&str)->WorkflowResult<archon_workflow::WorkflowAgentOutcome>{unreachable!()}
        async fn run_agent(&self,_:archon_workflow::WorkflowAgentCall)->WorkflowResult<archon_workflow::WorkflowAgentOutcome>{
            let records=archon_workflow::v2::record_landing::current().expect("actual dispatch installed record scope");
            assert_eq!(archon_tools::audit_landing::current().unwrap().tool_name(),"land-review-record");
            records.land(json!({"subject":"UNIT-1","findings":[{"id":"F1","claim":"counterexample"}],"evidence":[{"kind":"inspection","summary":"inspected source"}]}))?;
            Ok(archon_workflow::WorkflowAgentOutcome {content:json!({"status":"accepted","data":{"records_landed":1}}).to_string(),tool_uses:vec![],tokens_in:0,tokens_out:0,stop_reason:Some("end_turn".into())})
        }
    }
    #[tokio::test]
    async fn typed_landing_actual_dispatch_reconstructs_review_without_changing_scope() {
        let temp=tempfile::tempdir().unwrap();let store=WorkflowV2ResultStore::new(temp.path().join("v2"));
        let (sink,_rx)=crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
        let client=LiveV2AgentClient::new(Arc::new(Port),sink,vec![],"fixture".into(),None,Some(30));
        let execution=WorkflowV2CallExecution {call:WorkflowV2HostCall {id:"review".into(),method:WorkflowV2HostMethod::Agent,write_mode:None,
            options:WorkflowV2HostOptions {extra:[("reviewContract".into(),json!({"stage":"map"}))].into(),..Default::default()}},input:json!({"canonical_task_ids":["UNIT-1"]}),depends_on:vec![]};
        let request=archon_workflow::v2::call_data::v2_agent_request("review",None,&execution,None);
        let result=run_v2_agent_call_with_rejected_output_log(&WorkflowV2AgentAdapter::new(),&client,&request,Some(&store)).await.unwrap();
        assert_eq!(result.data["findings"][0]["id"],"F1");
        assert_eq!(result.data["findings"][0]["canonical_task_ids"],json!(["UNIT-1"]));
        assert!(archon_tools::audit_landing::current().is_none());
    }
    #[test]
    fn skeleton_tool_schema_spells_out_the_task_entry() {
        use archon_tools::audit_landing::LandingHost;
        let temp=tempfile::tempdir().unwrap();
        let bridge=Bridge(Arc::new(RecordLanding::open(temp.path().into(),"id".into(),RecordKind::Skeleton,vec![],false).unwrap()));
        let schema=bridge.schema().unwrap();
        let task=&schema["properties"]["task"];
        assert_eq!(task["required"],json!(["task_id","file_name"]));
        for field in ["task_id","file_name","depends_on","blocks","implements","deliverable_contracts"] {assert!(task["properties"].get(field).is_some(),"{field} missing from {task}");}
        assert_eq!(task["properties"]["implements"]["items"]["type"],"string");
        assert_eq!(task["properties"]["deliverable_contracts"]["items"]["required"],json!(["kind","artifact_path"]));
        assert_eq!(task["properties"]["depends_on"]["items"]["properties"]["consumes"]["items"]["required"],json!(["artifact_path"]));
        assert!(task.get("additionalProperties").is_none(),"FrozenTask accepts unknown keys, so the schema must too");
        assert!(schema["description"].as_str().unwrap().contains("deliverable_contracts"));
    }
}
