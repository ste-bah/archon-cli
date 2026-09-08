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

struct SilentAssessor;
#[async_trait::async_trait]
impl WorkflowAgentDispatch for SilentAssessor {
 fn fanout_parallelism(&self,_:Option<usize>)->usize{1}
 async fn run_call(&self,_:&str,_:Option<String>,_:&WorkflowV2CallExecution,_:&WorkflowV2AgentAdapter,_:Option<&WorkflowV2ResultStore>,_:Option<&task_universe::WorkflowV2TaskUniverse>)->WorkflowResult<WorkflowV2Result>{std::future::pending().await}
}
#[tokio::test]
async fn configured_runtime_deadline_pauses_and_preserves_consumed_usage(){
 let t=tempfile::tempdir().unwrap();let store=WorkflowStore::project(t.path());
 let run=store.create_run(WorkflowSpec{schema:spec::WORKFLOW_SCHEMA.into(),name:"deadline".into(),task:"audit".into(),target_repository_root:None,max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![]}).unwrap();
 let runtime=AuditRuntime::initialize(store.clone(),run.id.clone(),AuditPolicy{attempt_timeout_secs:Limit::Finite(1),total_time_secs:Limit::Finite(2),unexpected_change_refreshes:Limit::Unlimited}).unwrap();
 let result=tokio::time::timeout(std::time::Duration::from_secs(4),runtime.assess(
   &Snapshot{identity:"one".into(),root:t.path().into(),paths:vec!["old.txt".into()]},&["new.txt".into()],"initial",&SilentAssessor)).await.unwrap();
 assert!(matches!(result,Err(WorkflowError::ControlPaused(_))));
 let state=runtime.state().unwrap();assert!(state.budget.spent_ms>=1000);assert!(state.budget.active.is_none());
 let reloaded=AuditRuntime::initialize(store,run.id,AuditPolicy{attempt_timeout_secs:Limit::Unlimited,total_time_secs:Limit::Unlimited,unexpected_change_refreshes:Limit::Unlimited}).unwrap();
 assert_eq!(reloaded.state().unwrap().budget.spent_ms,state.budget.spent_ms);
 assert_eq!(reloaded.state().unwrap().budget.policy.total_time_secs,Limit::Finite(2));
}

#[tokio::test]
async fn final_snapshot_changes_spend_refresh_allowance_and_name_changes() {
 let t=tempfile::tempdir().unwrap();let store=WorkflowStore::project(t.path());
 let run=store.create_run(WorkflowSpec{schema:spec::WORKFLOW_SCHEMA.into(),name:"refresh".into(),task:"audit".into(),target_repository_root:None,max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![]}).unwrap();
 let runtime=AuditRuntime::initialize(store,run.id,AuditPolicy{attempt_timeout_secs:Limit::Finite(10),total_time_secs:Limit::Unlimited,unexpected_change_refreshes:Limit::Finite(1)}).unwrap();
 let calls=Arc::new(AtomicUsize::new(0));let assessor=Assessor(calls);
 let snapshot=|id:&str|Snapshot{identity:id.into(),root:t.path().into(),paths:vec![]};
 runtime.assess(&snapshot("one"),&[],"initial",&assessor).await.unwrap();
 runtime.assess(&snapshot("two"),&[],"final",&assessor).await.unwrap();
 assert_eq!(runtime.state().unwrap().budget.unexpected_refreshes,1);
 assert!(matches!(runtime.assess(&snapshot("three"),&[],"final",&assessor).await,Err(WorkflowError::ControlPaused(_))));
}

#[tokio::test]
async fn same_snapshot_with_new_obligation_never_reuses_incomplete_assessment() {
 let t=tempfile::tempdir().unwrap();let store=WorkflowStore::project(t.path());
 let run=store.create_run(WorkflowSpec{schema:spec::WORKFLOW_SCHEMA.into(),name:"coverage".into(),task:"audit".into(),target_repository_root:None,max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![]}).unwrap();
 let audit=AuditRuntime::initialize(store,run.id,AuditPolicy{attempt_timeout_secs:Limit::Unlimited,total_time_secs:Limit::Unlimited,unexpected_change_refreshes:Limit::Unlimited}).unwrap();
 let calls=Arc::new(AtomicUsize::new(0));let assessor=Assessor(calls.clone());
 std::fs::write(t.path().join("old.txt"),"source").unwrap();
 let snapshot=Snapshot{identity:"one".into(),root:t.path().into(),paths:vec!["old.txt".into()]};
 audit.assess(&snapshot,&["new.txt".into()],"initial",&assessor).await.unwrap();
 audit.assess(&snapshot,&["second.txt".into()],"new_paths",&assessor).await.unwrap();
 assert_eq!(calls.load(Ordering::SeqCst),2);
 assert_eq!(audit.records_for(&["second.txt".into()]).unwrap().len(),1);
}

#[tokio::test]
async fn repository_audit_reassessment_is_consumed_once_on_same_snapshot() {
    use archon_workflow::repository_audit::ledger::Reassessment;
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(WorkflowSpec{schema:spec::WORKFLOW_SCHEMA.into(),name:"dispute".into(),task:"audit".into(),target_repository_root:None,max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![]}).unwrap();
    let audit = AuditRuntime::initialize(store,run.id,AuditPolicy{attempt_timeout_secs:Limit::Unlimited,total_time_secs:Limit::Unlimited,unexpected_change_refreshes:Limit::Unlimited}).unwrap();
    std::fs::write(temp.path().join("old.txt"),"implementation").unwrap();
    let snapshot=Snapshot{identity:"one".into(),root:temp.path().into(),paths:vec!["old.txt".into()]};
    let calls=Arc::new(AtomicUsize::new(0)); let assessor=Assessor(calls.clone());
    audit.assess(&snapshot,&["new.txt".into()],"initial",&assessor).await.unwrap();
    audit.update(|state| {state.ledger.reassessments.push(Reassessment{declared_path:"new.txt".into(),snapshot:"one".into(),action_id:"confirmed-action".into(),reason:"inspect alternate entry".into(),attempted:false});Ok(())}).unwrap();
    audit.assess(&snapshot,&["new.txt".into()],"dispatch",&assessor).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst),2,"pending dispute was hidden by snapshot cache");
    audit.assess(&snapshot,&["new.txt".into()],"dispatch",&assessor).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst),2,"same dispute consumed an unbounded extra assessment");
    assert!(audit.require_closed("one").is_err(),"a contrary reassessment must preserve the open finding");
    assert!(audit.state().unwrap().ledger.reassessments[0].attempted);
}

#[tokio::test]
async fn repository_audit_status_exposes_limits_usage_and_remaining_allowance() {
    let temp=tempfile::tempdir().unwrap();
    let store=WorkflowStore::project(temp.path());
    let run=store.create_run(WorkflowSpec{schema:spec::WORKFLOW_SCHEMA.into(),name:"status".into(),task:"audit".into(),target_repository_root:None,max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![]}).unwrap();
    let audit=AuditRuntime::initialize(store,run.id,AuditPolicy{attempt_timeout_secs:Limit::Unlimited,total_time_secs:Limit::Finite(20),unexpected_change_refreshes:Limit::Finite(3)}).unwrap();
    audit.update(|state| {state.budget.spent_ms=500;state.budget.unexpected_refreshes=2;Ok(())}).unwrap();
    let status=audit.status().unwrap();
    assert_eq!(status["attempt_timeout_secs"],"unlimited");
    assert_eq!(status["remaining_time_ms"],19500);
    assert_eq!(status["remaining_unexpected_refreshes"],1);
}

#[tokio::test]
async fn repository_audit_write_boundaries_are_serialized_until_apply_finishes() {
    let temp=tempfile::tempdir().unwrap();
    let store=WorkflowStore::project(temp.path());
    let run=store.create_run(WorkflowSpec{schema:spec::WORKFLOW_SCHEMA.into(),name:"boundary".into(),task:"audit".into(),target_repository_root:None,max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![]}).unwrap();
    let audit=AuditRuntime::initialize(store,run.id,AuditPolicy{attempt_timeout_secs:Limit::Unlimited,total_time_secs:Limit::Unlimited,unexpected_change_refreshes:Limit::Unlimited}).unwrap();
    let first=audit.lock_write_boundary().await;
    assert!(tokio::time::timeout(std::time::Duration::from_millis(20),audit.lock_write_boundary()).await.is_err(),"another wave can replace the assessment while a branch is using it");
    drop(first);
    tokio::time::timeout(std::time::Duration::from_secs(1),audit.lock_write_boundary()).await.unwrap();
}
