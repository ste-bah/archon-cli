//! Typed evidence records scoped to one host invocation, never repository writes.
use crate::{WorkflowError,WorkflowResult,WorkflowV2AgentRequest,WorkflowV2Result,WorkflowV2Status,WorkflowV2Evidence,WorkflowV2CommandRecord};
use serde::{Deserialize,Serialize};
use serde_json::{Value,json};
use std::{collections::BTreeMap,path::PathBuf,sync::{Arc,Mutex}};
#[derive(Clone,Copy,Debug,Serialize,Deserialize,PartialEq,Eq)]
pub enum RecordKind { Review, Verify, Skeleton }
#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageRecord {
    subject:String,
    #[serde(default)] findings:Vec<Value>,
    #[serde(default)] evidence:Vec<WorkflowV2Evidence>,
    #[serde(default)] commands_run:Vec<WorkflowV2CommandRecord>,
    #[serde(default)] status:Option<WorkflowV2Status>,
    #[serde(default)] summary:String,
    #[serde(default)] task:Option<crate::task_skeleton::FrozenTask>,
}
pub struct RecordLanding { root:PathBuf,kind:RecordKind,subjects:Vec<String>,lock:Mutex<()> }
fn invalid(e:impl std::fmt::Display)->WorkflowError {WorkflowError::ArtifactInvalid(e.to_string())}
tokio::task_local! { static RECORDS: Arc<RecordLanding>; }
pub async fn scope<T>(records:Arc<RecordLanding>,future:impl std::future::Future<Output=T>)->T {RECORDS.scope(records,future).await}
pub fn current()->Option<Arc<RecordLanding>> {RECORDS.try_with(Clone::clone).ok()}
impl RecordLanding {
    pub fn open(root:PathBuf,identity:String,kind:RecordKind,subjects:Vec<String>)->WorkflowResult<Self> {
        std::fs::create_dir_all(&root).map_err(invalid)?;
        let path=root.join("contract.json");let expected=json!({"identity":identity,"kind":kind,"subjects":subjects});
        if path.exists() {
            let saved:Value=serde_json::from_slice(&std::fs::read(&path).map_err(invalid)?)?;
            if saved!=expected {return Err(invalid("record landing identity changed"));}
        } else {write(&path,&expected)?;}
        Ok(Self {root,kind,subjects,lock:Mutex::new(())})
    }
    pub fn kind(&self)->RecordKind {self.kind}
    pub fn tool_name(&self)->&'static str {match self.kind {RecordKind::Review=>"land-review-record",RecordKind::Verify=>"land-verify-record",RecordKind::Skeleton=>"land-skeleton-record"}}
    fn validate(&self,r:&StageRecord)->WorkflowResult<()> {
        if r.subject.trim().is_empty() || r.subject.len()>4096 {return Err(invalid("record subject is missing or too long"));}
        if self.kind!=RecordKind::Skeleton && !self.subjects.contains(&r.subject) {return Err(invalid("record subject is outside this call"));}
        if self.kind==RecordKind::Skeleton {
            let task=r.task.as_ref().ok_or_else(||invalid("skeleton record requires task"))?;
            if task.task_id!=r.subject || task.file_name.is_empty() {return Err(invalid("skeleton task identity mismatch"));}
            crate::repository_audit::contract::validate_path(&task.file_name).map_err(invalid)?;
            return Ok(());
        }
        if r.evidence.is_empty() || r.evidence.iter().any(|e|e.summary.trim().is_empty()) {return Err(invalid("record requires concrete evidence"));}
        if r.findings.len()>25 || r.findings.iter().any(|f| !f.is_object() || !["claim","summary","finding","title"].iter().any(|key|f.get(key).and_then(Value::as_str).is_some_and(|s|!s.trim().is_empty()))) {
            return Err(invalid("findings must be at most 25 structured findings with a nonempty claim/summary/finding/title"));
        }
        if self.kind==RecordKind::Verify && !matches!(r.status,Some(WorkflowV2Status::Accepted|WorkflowV2Status::Noop|WorkflowV2Status::Failed|WorkflowV2Status::Blocked|WorkflowV2Status::NeedsReview)) {
            return Err(invalid("verify record needs an explicit terminal verdict"));
        }
        Ok(())
    }
    pub fn land(&self,value:Value)->WorkflowResult<()> {
        let _lock=self.lock.lock().map_err(invalid)?;
        let record:StageRecord=serde_json::from_value(value)?;self.validate(&record)?;
        use sha2::{Digest,Sha256};
        write(&self.root.join(format!("record-{:x}.json",Sha256::digest(record.subject.as_bytes()))),&record)
    }
    fn records(&self)->WorkflowResult<BTreeMap<String,StageRecord>> {
        let mut records=BTreeMap::new();
        for entry in std::fs::read_dir(&self.root).map_err(invalid)? {
            let path=entry.map_err(invalid)?.path();
            if !path.file_name().and_then(|n|n.to_str()).is_some_and(|s|s.starts_with("record-") && s.ends_with(".json")){continue;}
            let record:StageRecord=serde_json::from_slice(&std::fs::read(path).map_err(invalid)?)?;
            self.validate(&record)?;records.insert(record.subject.clone(),record);
        }
        Ok(records)
    }
    pub fn remaining(&self)->WorkflowResult<Vec<String>> {
        let records=self.records()?;
        Ok(self.subjects.iter().filter(|id|!records.contains_key(*id)).cloned().collect())
    }
    pub fn hint(&self)->WorkflowResult<String> {
        let records=self.records()?;
        Ok(format!("Host retained {} records. Landed subjects: {}. Remaining subjects: {}. Use {} with {{subject,findings,evidence,commands_run,status,summary,task}} (task only for skeleton). Land an empty findings array plus evidence for a checked clean subject. Final data.records_landed must equal the complete saved count. Do not re-gather landed subjects. Skeleton subjects are task ids you establish; final raw skeleton may contain records_landed instead of tasks. All original final validation still applies.",
            records.len(),serde_json::to_string(&records.keys().collect::<Vec<_>>())?,serde_json::to_string(&self.remaining()?)?,self.tool_name()))
    }
    pub fn assemble(&self,completion:&Value)->WorkflowResult<Value> {
        let _lock=self.lock.lock().map_err(invalid)?;let records=self.records()?;
        let missing=self.subjects.iter().filter(|id|!records.contains_key(*id)).collect::<Vec<_>>();
        if !missing.is_empty(){return Err(invalid(format!("missing record subjects: {missing:?}")));}
        if completion.get("records_landed").and_then(Value::as_u64)!=Some(records.len() as u64){return Err(invalid("records_landed does not match retained evidence"));}
        let rows=records.values().collect::<Vec<_>>();
        let findings=rows.iter().flat_map(|r|r.findings.iter().cloned().map(|mut finding| {
            if let Some(object)=finding.as_object_mut() { object.entry("canonical_task_ids").or_insert_with(||json!([r.subject])); }
            finding
        })).collect::<Vec<_>>();
        Ok(json!({"findings":findings,"verification_records":rows,"tasks":rows.iter().filter_map(|r|r.task.as_ref()).collect::<Vec<_>>()}))
    }
    pub fn expand(&self,result:&mut WorkflowV2Result)->WorkflowResult<()> {
        if result.data.get("records_landed").is_none(){return Ok(());}
        let data=self.assemble(&result.data)?;
        result.data["findings"]=data["findings"].clone();
        if self.kind==RecordKind::Verify {result.data["verification_records"]=data["verification_records"].clone();}
        for row in data["verification_records"].as_array().into_iter().flatten() {
            let record:StageRecord=serde_json::from_value(row.clone())?;
            result.evidence.extend(record.evidence);result.commands_run.extend(record.commands_run);
            if self.kind==RecordKind::Verify && matches!(record.status,Some(WorkflowV2Status::Failed|WorkflowV2Status::Blocked|WorkflowV2Status::NeedsReview)) {result.status=WorkflowV2Status::NeedsReview;}
        }
        if result.summary.trim().is_empty(){result.summary="Host assembled landed evidence records".into();}
        Ok(())
    }
}
fn write(path:&std::path::Path,value:&impl Serialize)->WorkflowResult<()> {
    let tmp=path.with_extension(format!("{}.tmp",uuid::Uuid::new_v4()));
    std::fs::write(&tmp,serde_json::to_vec(value)?).map_err(invalid)?;
    std::fs::rename(tmp,path).map_err(invalid)
}
pub(crate) fn expand_result(_request:&WorkflowV2AgentRequest,result:&mut WorkflowV2Result)->Result<(),crate::WorkflowV2AgentError> {
    if let Some(records)=current(){ records.expand(result).map_err(|e|crate::WorkflowV2AgentError::InvalidResult(e.to_string()))?; }
    Ok(())
}
