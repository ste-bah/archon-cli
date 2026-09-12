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
    if kind==RecordKind::Skeleton {subjects.clear();}
    else if subjects.is_empty(){subjects.push(request.call.id.clone());}
    subjects.sort();subjects.dedup();
    use sha2::{Digest,Sha256};
    let identity=format!("{:x}",Sha256::digest(serde_json::to_vec(request)?));
    let records=Arc::new(RecordLanding::open(store.root().join("stage-records").join(&identity),identity,kind,subjects)?);
    request.task.push_str(&format!("\n{}",records.hint()?));
    Ok(Some(records))
}
struct Bridge(Arc<RecordLanding>);
impl archon_tools::audit_landing::LandingHost for Bridge {
    fn tool_name(&self)->&'static str {self.0.tool_name()}
    fn schema(&self)->Option<Value>{Some(json!({"type":"object","required":["subject"],"additionalProperties":false,"properties":{
        "subject":{"type":"string"},"findings":{"type":"array","items":{"type":"object"}},"evidence":{"type":"array","items":{"type":"object"}},
        "commands_run":{"type":"array","items":{"type":"object"}},"status":{"type":"string"},"summary":{"type":"string"},"task":{"type":"object"}}}))}
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
