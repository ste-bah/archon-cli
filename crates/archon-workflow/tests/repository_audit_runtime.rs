use archon_workflow::*;
use archon_workflow::repository_audit::{budget::{AuditPolicy,Limit},runtime::{AuditRuntime,Snapshot}};
use serde_json::json;
use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};

struct Assessor(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl WorkflowAgentDispatch for Assessor {
 fn fanout_parallelism(&self,_:Option<usize>)->usize{1}
 async fn run_call(&self,_:&str,_:Option<String>,e:&WorkflowV2CallExecution,a:&WorkflowV2AgentAdapter,_:Option<&WorkflowV2ResultStore>,_:Option<&task_universe::WorkflowV2TaskUniverse>)->WorkflowResult<WorkflowV2Result>{
  self.0.fetch_add(1,Ordering::SeqCst);
  let c=&e.call.options.extra["repository_audit_contract"];
  let records=c["declared_paths"].as_array().unwrap().iter().map(|p|json!({"declared_path":p,"verdict":"exists_elsewhere","equivalents":["old.txt"],"required_action":"wire_or_migrate","reason":"same behavior in old location"})).collect::<Vec<_>>();
  let output=json!({"status":"accepted","summary":"audited","evidence":[{"kind":"inspection","summary":"read source"}],"data":{"repository_audit":{"schema_version":1,"snapshot":c["snapshot"],"records":records}}});
  a.parse_agent_output(&v2::call_data::v2_agent_request("audit",None,e,None),&output.to_string()).map_err(|e|WorkflowError::StageFailed(e.to_string()))
 }
}
#[tokio::test]
async fn audit_runtime_persists_assessment_reuses_snapshot_and_blocks_open_findings(){
 let t=tempfile::tempdir().unwrap();let project=t.path().join("project");let store=WorkflowStore::project(&project);
 let run=store.create_run(WorkflowSpec{schema:spec::WORKFLOW_SCHEMA.into(),name:"audit".into(),task:"audit".into(),target_repository_root:None,max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![]}).unwrap();
 let runtime=AuditRuntime::initialize(store.clone(),run.id.clone(),AuditPolicy{attempt_timeout_secs:Limit::Finite(7200),total_time_secs:Limit::Unlimited,unexpected_change_refreshes:Limit::Finite(12)}).unwrap();
 let source=t.path().join("source");std::fs::create_dir(&source).unwrap();std::fs::write(source.join("old.txt"),"implementation").unwrap();
 let snapshot=Snapshot{identity:"one".into(),root:source,paths:vec!["old.txt".into()]};
 let calls=Arc::new(AtomicUsize::new(0));let assessor=Assessor(calls.clone());
 runtime.assess(&snapshot,&["new.txt".into()],"initial",&assessor).await.unwrap();
 runtime.assess(&snapshot,&["new.txt".into()],"dispatch",&assessor).await.unwrap();
 assert_eq!(calls.load(Ordering::SeqCst),1);
 assert_eq!(runtime.records_for(&["new.txt".into()]).unwrap().len(),1);
 assert!(runtime.records_for(&["other.txt".into()]).unwrap().is_empty());
 assert!(runtime.require_closed("one").is_err());
 assert!(store.run_dir(&run.id).join("v2/repository-audit/state.json").exists());
}

#[tokio::test]
async fn empty_repository_records_audit_without_calling_provider() {
 let t=tempfile::tempdir().unwrap();let store=WorkflowStore::project(t.path());
 let run=store.create_run(WorkflowSpec{schema:spec::WORKFLOW_SCHEMA.into(),name:"empty-audit".into(),task:"audit".into(),target_repository_root:None,max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![]}).unwrap();
 let runtime=AuditRuntime::initialize(store,run.id,AuditPolicy{attempt_timeout_secs:Limit::Unlimited,total_time_secs:Limit::Unlimited,unexpected_change_refreshes:Limit::Unlimited}).unwrap();
 let calls=Arc::new(AtomicUsize::new(0));
 runtime.assess(&Snapshot{identity:"empty".into(),root:t.path().into(),paths:vec![]},&[],"initial",&Assessor(calls.clone())).await.unwrap();
 assert_eq!(calls.load(Ordering::SeqCst),0);
 runtime.require_closed("empty").unwrap();
}
