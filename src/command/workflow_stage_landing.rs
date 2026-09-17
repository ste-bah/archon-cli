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
    // Issue-39: one schema for all three kinds said only `subject` was required and left every item a bare object, so the model learned the contract by being rejected; build it per kind from the enums `validate` enforces.
    fn schema(&self)->Option<Value>{
        let (kind,names)=(self.0.kind(),archon_workflow::v2::record_landing::enum_names());
        let required=match kind {RecordKind::Review=>json!(["subject","evidence","summary"]),RecordKind::Verify=>json!(["subject","evidence","summary","status"]),RecordKind::Skeleton=>json!(["subject","evidence","summary","task"])};
        let mut properties=json!({
            "subject":{"type":"string","description":"One of this call's subjects"},
            "evidence":{"type":"array","minItems":1,"items":{"type":"object","required":["kind","summary"],"properties":{"kind":{"type":"string","enum":names.evidence_kinds},"summary":{"type":"string"},"source":{"type":"string"}}}},
            "commands_run":{"type":"array","items":{"type":"object","required":["kind","command","status","output_summary"],"properties":{
                "kind":{"type":"string","enum":names.command_kinds},"command":{"type":"string"},"status":{"type":"string","enum":names.command_statuses},
                "exit_code":{"type":"integer"},"output_summary":{"type":"string","description":"Captured output, not a placeholder"},"pre_existing":{"type":"boolean"}}}},
            "status":{"type":"string","enum":names.verify_verdicts},"summary":{"type":"string"},
            "replace":{"type":"boolean","description":"Supersede the earlier record for this subject instead of unioning with it"}});
        if kind==RecordKind::Verify {properties["status"]["description"]=json!("Terminal verdict; accepted/noop needs a succeeded commands_run entry with captured output");}
        if kind!=RecordKind::Skeleton {properties["findings"]=json!({"type":"array","maxItems":25,"items":{"type":"object","anyOf":[{"required":["claim"]},{"required":["summary"]},{"required":["finding"]},{"required":["title"]}],
            "properties":{"claim":{"type":"string"},"summary":{"type":"string"},"finding":{"type":"string"},"title":{"type":"string"},"task_id":{"type":"string"},"task_ids":{"type":"array","items":{"type":"string"}},"canonical_task_ids":{"type":"array","items":{"type":"string"}},"attributable_to_task":{"type":"boolean"},"file_name":{"type":"string"},"severity":{"type":"string"},"kind":{"type":"string"},"evidence":{}}}});}
        // Issue-38: `FrozenTask` deserialises with serde defaults and accepts unknown keys, so `task` stays open; the listed properties are the fields the host reads.
        if kind==RecordKind::Skeleton {properties["task"]=json!({"type":"object","description":"Skeleton records only: one full skeleton entry","required":["task_id","file_name"],"properties":{
            "task_id":{"type":"string"},"file_name":{"type":"string"},
            "depends_on":{"type":"array","items":{"type":"object","required":["task_id"],"properties":{"task_id":{"type":"string"},"consumes":{"type":"array","items":{"type":"object","required":["artifact_path"],"properties":{"artifact_path":{"type":"string"}}}},"ordering_only":{"type":"boolean"}}}},
            "blocks":{"type":"array","items":{"type":"string"}},"implements":{"type":"array","items":{"type":"string","minLength":1}},
            "deliverable_contracts":{"type":"array","items":{"type":"object","required":["kind","artifact_path"],"properties":{"kind":{"type":"string","minLength":1},"artifact_path":{"type":"string","minLength":1},"min_instances":{"type":"integer"}}}}}});}
        Some(json!({"type":"object","required":required,"additionalProperties":false,"description":archon_workflow::v2::record_landing::schema_hint(),"properties":properties}))
    }
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
    fn schema_for(kind:RecordKind)->Value {
        use archon_tools::audit_landing::LandingHost;
        let temp=tempfile::tempdir().unwrap();
        Bridge(Arc::new(RecordLanding::open(temp.path().into(),"id".into(),kind,vec![],false).unwrap())).schema().unwrap()
    }
    #[test]
    fn skeleton_tool_schema_spells_out_the_task_entry() {
        let schema=schema_for(RecordKind::Skeleton);
        let task=&schema["properties"]["task"];
        assert_eq!(task["required"],json!(["task_id","file_name"]));
        for field in ["task_id","file_name","depends_on","blocks","implements","deliverable_contracts"] {assert!(task["properties"].get(field).is_some(),"{field} missing from {task}");}
        assert_eq!(task["properties"]["implements"]["items"]["type"],"string");
        assert_eq!(task["properties"]["deliverable_contracts"]["items"]["required"],json!(["kind","artifact_path"]));
        assert_eq!(task["properties"]["depends_on"]["items"]["properties"]["consumes"]["items"]["required"],json!(["artifact_path"]));
        assert!(task.get("additionalProperties").is_none(),"FrozenTask accepts unknown keys, so the schema must too");
        assert!(schema["description"].as_str().unwrap().contains("deliverable_contracts"));
    }
    // Issue-39: the schema the model sees is built per kind and states what `validate` enforces.
    #[test]
    fn tool_schema_required_lists_and_enums_follow_the_record_kind() {
        let names=archon_workflow::v2::record_landing::enum_names();
        for (kind,required) in [(RecordKind::Review,json!(["subject","evidence","summary"])),(RecordKind::Verify,json!(["subject","evidence","summary","status"])),(RecordKind::Skeleton,json!(["subject","evidence","summary","task"]))] {
            let schema=schema_for(kind);
            assert_eq!(schema["required"],required,"{kind:?}");
            assert_eq!(schema["additionalProperties"],json!(false));
            let evidence=&schema["properties"]["evidence"]["items"];
            assert_eq!(evidence["required"],json!(["kind","summary"]),"{kind:?}");
            assert_eq!(evidence["properties"]["kind"]["enum"],json!(names.evidence_kinds),"{kind:?}");
            assert!(!evidence["properties"]["kind"]["enum"].as_array().unwrap().is_empty());
            let command=&schema["properties"]["commands_run"]["items"];
            assert_eq!(command["required"],json!(["kind","command","status","output_summary"]),"{kind:?}");
            assert_eq!(command["properties"]["kind"]["enum"],json!(names.command_kinds));
            assert_eq!(command["properties"]["status"]["enum"],json!(names.command_statuses));
            assert_eq!(schema["properties"]["status"]["enum"],json!(names.verify_verdicts),"{kind:?}");
            assert_eq!(schema["properties"]["status"]["enum"],json!(["accepted","noop","failed","blocked","needs_review"]));
            for key in schema["required"].as_array().unwrap() {assert!(schema["properties"].get(key.as_str().unwrap()).is_some(),"{kind:?} requires {key} but does not describe it");}
        }
        let (review,verify,skeleton)=(schema_for(RecordKind::Review),schema_for(RecordKind::Verify),schema_for(RecordKind::Skeleton));
        assert!(review["properties"].get("task").is_none() && verify["properties"].get("task").is_none(),"task is a skeleton field");
        assert!(skeleton["properties"].get("findings").is_none(),"findings are not skeleton fields");
        let finding=&review["properties"]["findings"]["items"];
        assert_eq!(finding["anyOf"],json!([{"required":["claim"]},{"required":["summary"]},{"required":["finding"]},{"required":["title"]}]));
        assert_eq!(verify["properties"]["findings"]["items"]["anyOf"],finding["anyOf"]);
        assert!(verify["properties"]["status"]["description"].as_str().unwrap().contains("succeeded commands_run entry"));
        assert_eq!(skeleton["properties"]["task"]["properties"]["deliverable_contracts"]["items"]["properties"]["artifact_path"]["minLength"],json!(1));
    }
}
