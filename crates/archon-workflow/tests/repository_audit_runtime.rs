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

#[tokio::test]
async fn repository_audit_explicit_reassessment_corrects_mistaken_judgment_without_fabricating_apply() {
    struct Corrected;
    #[async_trait::async_trait]
    impl WorkflowAgentDispatch for Corrected {
        fn fanout_parallelism(&self,_:Option<usize>)->usize{1}
        async fn run_call(&self,_:&str,_:Option<String>,e:&WorkflowV2CallExecution,_:&WorkflowV2AgentAdapter,_:Option<&WorkflowV2ResultStore>,_:Option<&task_universe::WorkflowV2TaskUniverse>)->WorkflowResult<WorkflowV2Result>{
            let mut result=WorkflowV2Result::accepted("reassessed actual entry point");
            result.data=json!({"repository_audit":{"schema_version":1,"snapshot":e.input["snapshot"],"records":[{
                "declared_path":"entry.txt","verdict":"exists_as_declared","equivalents":[],"required_action":"none","reason":"existing entry point is connected"
            }]},"audit_corrections":[{"declared_path":"entry.txt","snapshot":"one","action_id":"dispute","reason":"prior reachability judgment overlooked entry point","evidence_paths":["entry.txt"]}]});
            Ok(result)
        }
    }
    use archon_workflow::repository_audit::{AuditContract,AuditReport,ledger::Reassessment};
    let temp=tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("entry.txt"),"connected implementation").unwrap();
    let store=WorkflowStore::project(temp.path());
    let run=store.create_run(WorkflowSpec{schema:spec::WORKFLOW_SCHEMA.into(),name:"correction".into(),task:"audit".into(),target_repository_root:None,max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![]}).unwrap();
    let audit=AuditRuntime::initialize(store,run.id,AuditPolicy{attempt_timeout_secs:Limit::Unlimited,total_time_secs:Limit::Unlimited,unexpected_change_refreshes:Limit::Unlimited}).unwrap();
    let snapshot=Snapshot{identity:"one".into(),root:temp.path().into(),paths:vec!["entry.txt".into()]};
    audit.update(|s| {
        s.snapshot=Some(snapshot.clone());s.declared_paths.insert("entry.txt".into());
        let report:AuditReport=serde_json::from_value(json!({"schema_version":1,"snapshot":"one","records":[{"declared_path":"entry.txt","verdict":"unreachable","equivalents":[],"required_action":"wire_or_migrate","reason":"initial mistaken judgment"}]}))?;
        s.ledger.accept(AuditContract{schema_version:1,snapshot:"one".into(),declared_paths:vec!["entry.txt".into()]},report)?;
        s.ledger.reassessments.push(Reassessment{declared_path:"entry.txt".into(),snapshot:"one".into(),action_id:"dispute".into(),reason:"counterevidence".into(),attempted:false});Ok(())
    }).unwrap();
    audit.assess(&snapshot,&["entry.txt".into()],"dispatch",&Corrected).await.unwrap();
    audit.require_closed("one").expect("explicit assessed correction must close the mistaken finding");
    let state=audit.state().unwrap();
    assert!(state.ledger.obligations["entry.txt"].applied_commit.is_none());
    assert_eq!(state.ledger.history.len(),2);
}

/// Issue-25: after an interrupted post-apply audit the tree is a receipt's
/// `after`; the resume-time "initial" audit must not spend the allowance.
#[tokio::test]
async fn snapshot_named_by_an_apply_receipt_is_not_an_unexpected_refresh() {
 let t=tempfile::tempdir().unwrap();let store=WorkflowStore::project(t.path());
 let run=store.create_run(WorkflowSpec{schema:spec::WORKFLOW_SCHEMA.into(),name:"receipted".into(),task:"audit".into(),target_repository_root:None,max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![]}).unwrap();
 let runtime=AuditRuntime::initialize(store.clone(),run.id.clone(),AuditPolicy{attempt_timeout_secs:Limit::Unlimited,total_time_secs:Limit::Unlimited,unexpected_change_refreshes:Limit::Finite(0)}).unwrap();
 let calls=Arc::new(AtomicUsize::new(0));let assessor=Assessor(calls);
 let snapshot=|id:&str|Snapshot{identity:id.into(),root:t.path().into(),paths:vec![]};
 runtime.assess(&snapshot("x"),&[],"initial",&assessor).await.unwrap();
 store.write_run_json(&run.id,"v2/repository-audit/apply-agents-1-0.json",&json!({"commit":"c1","items_applied":["agents-1-0"],"before":"x","after":"y","unexpected_paths":[]})).unwrap();
 runtime.assess(&snapshot("y"),&[],"initial",&assessor).await.unwrap();
 assert_eq!(runtime.state().unwrap().budget.unexpected_refreshes,0);
 assert_eq!(runtime.state().unwrap().snapshot.unwrap().identity,"y");
 let events=std::fs::read_to_string(store.events_path(&run.id)).unwrap();
 let started=events.lines().map(|l|serde_json::from_str::<serde_json::Value>(l).unwrap()).filter(|r|r["detail"]["event"]=="repository_audit_started").collect::<Vec<_>>();
 assert_eq!(started.last().unwrap()["detail"]["receipted_commit"],"c1","{started:#?}");
 assert!(matches!(runtime.assess(&snapshot("z"),&[],"initial",&assessor).await,Err(WorkflowError::ControlPaused(_))));
 assert!(matches!(runtime.assess(&snapshot("w"),&[],"unexpected_change",&assessor).await,Err(WorkflowError::ControlPaused(_))),"explicit foreign paths stay charged");
}

// Issue-51: an attempt lands only the paths that need a fresh verdict; every
// other declared path carries its last record forward verbatim.
mod carry {
 use super::*;
 use archon_workflow::repository_audit::{AuditContract,AuditRecord,AuditReport,RequiredAction,Verdict,ledger::{Reassessment,Waiver}};
 use std::{path::{Path,PathBuf},sync::Mutex};
 /// Records every contract it is handed; judges existence in the sealed root.
 struct Recording(Mutex<Vec<Vec<String>>>);
 #[async_trait::async_trait]
 impl WorkflowAgentDispatch for Recording {
  fn fanout_parallelism(&self,_:Option<usize>)->usize{1}
  async fn run_call(&self,_:&str,root:Option<String>,e:&WorkflowV2CallExecution,a:&WorkflowV2AgentAdapter,_:Option<&WorkflowV2ResultStore>,_:Option<&task_universe::WorkflowV2TaskUniverse>)->WorkflowResult<WorkflowV2Result>{
   let c:AuditContract=serde_json::from_value(e.call.options.extra["repository_audit_contract"].clone())?;
   self.0.lock().unwrap().push(c.declared_paths.clone());
   let root=PathBuf::from(root.unwrap());
   let records=c.declared_paths.iter().map(|p|{let exists=root.join(p).exists();json!({"declared_path":p,"verdict":if exists{"exists_as_declared"}else{"absent"},"equivalents":[],"required_action":if exists{"none"}else{"deliver"},"reason":"fresh verdict"})}).collect::<Vec<_>>();
   let output=json!({"status":"accepted","summary":"audited","evidence":[{"kind":"inspection","summary":"read source"}],"data":{"repository_audit":{"schema_version":1,"snapshot":c.snapshot,"records":records}}});
   a.parse_agent_output(&v2::call_data::v2_agent_request("audit",None,e,None),&output.to_string()).map_err(|e|WorkflowError::StageFailed(e.to_string()))
  }
 }
 fn seeded(path:&str,verdict:Verdict)->AuditRecord{
  let action=match verdict{Verdict::ExistsAsDeclared=>RequiredAction::None,Verdict::Absent=>RequiredAction::Deliver,_=>RequiredAction::WireOrMigrate};
  AuditRecord{declared_path:path.into(),verdict,equivalents:vec![],required_action:action,reason:"seeded verdict".into()}
 }
 fn files(dir:&Path,n:usize)->Vec<String>{std::fs::create_dir_all(dir).unwrap();(0..n).map(|i|{let p=format!("f{i}.txt");std::fs::write(dir.join(&p),format!("body {i}")).unwrap();p}).collect()}
 /// `n` declared files under `source`, all judged at snapshot "one"; `open` are seeded unreachable (an open obligation).
 fn prior(t:&Path,n:usize,open:&[&str])->(AuditRuntime,Snapshot,Vec<String>){
  let source=t.join("source");let paths=files(&source,n);let store=WorkflowStore::project(t.join("project"));
  let run=store.create_run(WorkflowSpec{schema:spec::WORKFLOW_SCHEMA.into(),name:"carry".into(),task:"audit".into(),target_repository_root:None,max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![]}).unwrap();
  let audit=AuditRuntime::initialize(store,run.id,AuditPolicy{attempt_timeout_secs:Limit::Unlimited,total_time_secs:Limit::Unlimited,unexpected_change_refreshes:Limit::Unlimited}).unwrap();
  let snapshot=Snapshot{identity:"one".into(),root:source,paths:paths.clone()};
  audit.update(|s|{
   s.declared_paths.extend(paths.iter().cloned());s.snapshot=Some(snapshot.clone());
   let records=paths.iter().map(|p|seeded(p,if open.contains(&p.as_str()){Verdict::Unreachable}else{Verdict::ExistsAsDeclared})).collect();
   s.ledger.accept(AuditContract{schema_version:1,snapshot:"one".into(),declared_paths:paths.clone()},AuditReport{schema_version:1,snapshot:"one".into(),records})
  }).unwrap();
  (audit,snapshot,paths)
 }
 fn contracts(a:&Recording)->Vec<Vec<String>>{a.0.lock().unwrap().clone()}
 #[tokio::test]
 async fn same_snapshot_lands_only_the_added_path_and_seals_the_merged_report(){
  let t=tempfile::tempdir().unwrap();let (audit,mut snapshot,_)=prior(t.path(),100,&[]);
  std::fs::write(snapshot.root.join("f100.txt"),"new").unwrap();snapshot.paths.push("f100.txt".into());
  let assessor=Recording(Mutex::new(vec![]));
  audit.assess(&snapshot,&["f100.txt".into()],"new_paths",&assessor).await.unwrap();
  assert_eq!(contracts(&assessor),vec![vec!["f100.txt".to_string()]]);
  let state=audit.state().unwrap();let last=state.ledger.history.last().unwrap();
  assert_eq!(state.ledger.history.len(),2);assert_eq!(last.records.len(),101);
  assert_eq!(audit.records_for(&["f0.txt".into()]).unwrap(),vec![seeded("f0.txt",Verdict::ExistsAsDeclared)],"carried verbatim");
  assert_eq!(audit.records_for(&["f100.txt".into()]).unwrap()[0].reason,"fresh verdict");
  audit.seal_final("one").expect("merged report covers every declared path");
  let events=std::fs::read_to_string(audit.store.events_path(&audit.run_id)).unwrap();
  let started=events.lines().map(|l|serde_json::from_str::<serde_json::Value>(l).unwrap()).find(|r|r["detail"]["event"]=="repository_audit_started").unwrap();
  assert_eq!(started["detail"]["carried_forward"],100);assert_eq!(started["detail"]["landing_paths"],json!(["f100.txt"]));
  assert_eq!(started["detail"]["declared_paths"].as_array().unwrap().len(),101);
 }
 #[tokio::test]
 async fn new_snapshot_lands_changed_and_added_paths_and_carries_the_rest(){
  let t=tempfile::tempdir().unwrap();let (audit,one,paths)=prior(t.path(),100,&[]);
  let next=t.path().join("next");files(&next,100);
  for p in ["f1.txt","f2.txt"]{std::fs::write(next.join(p),"edited").unwrap();}
  std::fs::write(next.join("f100.txt"),"new").unwrap();
  let mut two_paths=paths.clone();two_paths.push("f100.txt".into());
  let two=Snapshot{identity:"two".into(),root:next,paths:two_paths};
  let assessor=Recording(Mutex::new(vec![]));
  audit.assess(&two,&["f100.txt".into()],"post_apply",&assessor).await.unwrap();
  assert_eq!(contracts(&assessor),vec![vec!["f1.txt".to_string(),"f100.txt".into(),"f2.txt".into()]]);
  let state=audit.state().unwrap();let last=state.ledger.history.last().unwrap();
  assert_eq!(last.snapshot,"two");assert_eq!(last.records.len(),101);
  assert_eq!(state.snapshot.unwrap().identity,"two");
  assert_eq!(audit.records_for(&["f1.txt".into()]).unwrap()[0].reason,"fresh verdict");
  assert_eq!(audit.records_for(&["f3.txt".into()]).unwrap(),vec![seeded("f3.txt",Verdict::ExistsAsDeclared)]);
  assert!(one.root.exists(),"the previous view is what changes are read from");
  audit.seal_final("two").unwrap();
 }
 #[tokio::test]
 async fn pending_reassessment_path_is_in_the_delta_even_when_unchanged(){
  let t=tempfile::tempdir().unwrap();let (audit,snapshot,_)=prior(t.path(),20,&["f5.txt"]);
  audit.update(|s|{s.ledger.reassessments.push(Reassessment{declared_path:"f5.txt".into(),snapshot:"one".into(),action_id:"dispute".into(),reason:"look again".into(),attempted:false});Ok(())}).unwrap();
  let assessor=Recording(Mutex::new(vec![]));
  audit.assess(&snapshot,&[],"dispatch",&assessor).await.unwrap();
  assert_eq!(contracts(&assessor),vec![vec!["f5.txt".to_string()]]);
  let state=audit.state().unwrap();
  assert!(state.ledger.reassessments[0].attempted);assert_eq!(state.ledger.history.last().unwrap().records.len(),20);
  assert_eq!(audit.records_for(&["f4.txt".into()]).unwrap(),vec![seeded("f4.txt",Verdict::ExistsAsDeclared)]);
 }
 #[tokio::test]
 async fn unresolved_obligation_without_a_prior_record_is_in_the_delta(){
  let t=tempfile::tempdir().unwrap();let (audit,snapshot,_)=prior(t.path(),20,&["f6.txt"]);
  // Declared and obligated, but the last report never covered it (an interrupted attempt).
  audit.update(|s|{s.declared_paths.insert("extra.txt".into());let mut o=s.ledger.obligations["f6.txt"].clone();o.resolved_snapshot=None;s.ledger.obligations.insert("extra.txt".into(),o);Ok(())}).unwrap();
  let assessor=Recording(Mutex::new(vec![]));
  audit.assess(&snapshot,&[],"dispatch",&assessor).await.unwrap();
  assert_eq!(contracts(&assessor),vec![vec!["extra.txt".to_string()]],"f6 has a record at this snapshot and no new evidence: carried");
  assert_eq!(audit.records_for(&["f6.txt".into()]).unwrap(),vec![seeded("f6.txt",Verdict::Unreachable)]);
  assert_eq!(audit.state().unwrap().ledger.history.last().unwrap().records.len(),21);
 }
 #[tokio::test]
 async fn open_obligation_is_rejudged_at_a_new_snapshot_even_when_its_file_is_unchanged(){
  let t=tempfile::tempdir().unwrap();let (audit,_,paths)=prior(t.path(),20,&["f6.txt"]);
  let next=t.path().join("next");files(&next,20);std::fs::write(next.join("f0.txt"),"wired the entry point").unwrap();
  let two=Snapshot{identity:"two".into(),root:next,paths};
  let assessor=Recording(Mutex::new(vec![]));
  audit.assess(&two,&[],"post_apply",&assessor).await.unwrap();
  assert_eq!(contracts(&assessor),vec![vec!["f0.txt".to_string(),"f6.txt".into()]],"another file's edit may have satisfied the obligation");
  assert_eq!(audit.records_for(&["f6.txt".into()]).unwrap()[0].reason,"fresh verdict");
 }
 #[tokio::test]
 async fn waived_open_finding_is_carried_verbatim_and_its_waiver_untouched(){
  let t=tempfile::tempdir().unwrap();let (audit,mut snapshot,_)=prior(t.path(),20,&["f3.txt"]);
  audit.update(|s|{s.ledger.waivers.push(Waiver{declared_path:"f3.txt".into(),snapshot:"one".into(),action_id:"human-confirmed".into(),reason:"accepted exception".into(),assessment_count:1});Ok(())}).unwrap();
  assert!(audit.state().unwrap().ledger.unresolved("one").unwrap().is_empty(),"waived on the assessed report");
  std::fs::write(snapshot.root.join("f20.txt"),"new").unwrap();snapshot.paths.push("f20.txt".into());
  let assessor=Recording(Mutex::new(vec![]));
  audit.assess(&snapshot,&["f20.txt".into()],"new_paths",&assessor).await.unwrap();
  assert_eq!(contracts(&assessor),vec![vec!["f20.txt".to_string()]]);
  let state=audit.state().unwrap();
  assert_eq!(audit.records_for(&["f3.txt".into()]).unwrap(),vec![seeded("f3.txt",Verdict::Unreachable)],"a waiver never rewrites the judgment");
  assert_eq!(state.ledger.waivers.len(),1);assert_eq!(state.ledger.waivers[0].assessment_count,1);
  assert_eq!(state.ledger.history.last().unwrap().records.len(),21);
 }
 /// The live run's shape: hundreds declared, the last report at the same
 /// snapshot short of a couple of already-declared paths (an interrupted
 /// attempt), two open obligations with records. The next attempt lands the
 /// missing paths only.
 #[tokio::test]
 async fn resumed_state_with_hundreds_declared_lands_only_the_uncovered_paths(){
  let t=tempfile::tempdir().unwrap();let (audit,snapshot,_)=prior(t.path(),707,&["f10.txt","f11.txt"]);
  std::fs::write(snapshot.root.join("f707.txt"),"declared by the interrupted attempt").unwrap();
  audit.update(|s|{s.declared_paths.insert("f707.txt".into());s.attempts=15;Ok(())}).unwrap();
  let mut snapshot=snapshot;snapshot.paths.push("f707.txt".into());
  let assessor=Recording(Mutex::new(vec![]));
  audit.assess(&snapshot,&[],"initial",&assessor).await.unwrap();
  assert_eq!(contracts(&assessor),vec![vec!["f707.txt".to_string()]]);
  let state=audit.state().unwrap();
  assert_eq!(state.ledger.history.last().unwrap().records.len(),708);
  assert_eq!(state.ledger.unresolved("one").unwrap(),vec!["f10.txt".to_string(),"f11.txt".into()],"open findings carry, they do not close");
  let contract:serde_json::Value=serde_json::from_slice(&std::fs::read(audit.store.run_dir(&audit.run_id).join("v2/repository-audit/records/repository-audit-16/contract.json")).unwrap()).unwrap();
  assert_eq!(contract["contract"]["declared_paths"],json!(["f707.txt"]),"the landing expects one record, not 708");
 }
}
