//! Host-owned assessment lifecycle, durable across pauses and process restarts.
use crate::*;
use super::{budget::{AuditBudget, AuditPolicy}, ledger::AuditLedger, AuditContract, AuditRecord, AuditReport, RequiredAction, Verdict};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc, time::Duration};
use serde::{Serialize, Deserialize};
use serde_json::json;

pub const STATE_PATH: &str = "v2/repository-audit/state.json";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot { pub identity: String, pub root: PathBuf, pub paths: Vec<String> }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditState {
    pub schema_version: u32,
    pub generation: u64,
    pub budget: AuditBudget,
    pub ledger: AuditLedger,
    pub declared_paths: BTreeSet<String>,
    pub snapshot: Option<Snapshot>,
    pub attempts: u64,
    pub last_error: Option<String>,
    #[serde(default)]
    pub final_receipt: Option<FinalReceipt>,
    #[serde(default)]
    pub operator_controls: Vec<serde_json::Value>,
    #[serde(default)]
    pub policy_provenance: Option<serde_json::Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FinalReceipt {
    pub generation: u64,
    pub snapshot: String,
    pub declared_paths: BTreeSet<String>,
    pub assessment_count: usize,
}
impl AuditState {
    pub fn require_final_receipt(&self) -> WorkflowResult<()> {
        let snapshot = self.snapshot.as_ref().ok_or_else(||
            WorkflowError::StateCorrupt("repository audit final snapshot missing".into()))?;
        let expected = FinalReceipt { generation: self.generation,
            snapshot: snapshot.identity.clone(), declared_paths: self.declared_paths.clone(),
            assessment_count: self.ledger.history.len() };
        if self.final_receipt.as_ref() != Some(&expected) {
            return Err(WorkflowError::StateCorrupt("repository audit final assessment receipt missing or stale".into()));
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct AuditRuntime {
    pub store: WorkflowStore,
    pub run_id: String,
    pub generation: u64,
    assessment_lock: Arc<tokio::sync::Mutex<()>>,
    write_boundary_lock: Arc<tokio::sync::Mutex<()>>,
}
impl AuditRuntime {
    pub fn initialize(store: WorkflowStore, run_id: String, policy: AuditPolicy) -> WorkflowResult<Self> {
        let generation = store.load_state(&run_id)?.generation;
        store.with_run_lock(&run_id, |locked| {
            let path = locked.run_dir(&run_id).join(STATE_PATH);
            let required = locked.run_dir(&run_id).join("v2/repository-audit/required.json");
            if required.exists() && !path.exists() {
                return Err(WorkflowError::StateCorrupt("mandatory repository audit state is missing".into()));
            }
            if path.exists() {
                let mut state: AuditState = serde_json::from_slice(&std::fs::read(&path).map_err(|e| WorkflowError::io(&path,e))?)?;
                if state.schema_version != 1 { return Err(WorkflowError::StateCorrupt("unsupported audit state schema".into())); }
                if state.generation != generation {
                    state.budget.recover_interrupted(chrono::Utc::now().timestamp_millis())?;
                    state.generation = generation;
                    state.final_receipt = None;
                }
                locked.write_run_json(&run_id, STATE_PATH, &state)?;
            } else {
                locked.write_run_json(&run_id, STATE_PATH, &AuditState {
                    schema_version:1, generation, budget:AuditBudget::new(policy), ledger:AuditLedger::default(),
                    declared_paths:BTreeSet::new(), snapshot:None, attempts:0, last_error:None, final_receipt:None, operator_controls:vec![], policy_provenance:None,
                })?;
            }
            locked.write_run_json(&run_id, "v2/repository-audit/required.json", &json!({"schema_version":1}))?;
            Ok(())
        })?;
        Ok(Self { store, run_id, generation, assessment_lock:Arc::new(tokio::sync::Mutex::new(())), write_boundary_lock:Arc::new(tokio::sync::Mutex::new(())) })
    }
    /// Keep the assessed view stable until all branches have applied and been reassessed.
    pub async fn lock_write_boundary(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.write_boundary_lock.clone().lock_owned().await
    }
    pub fn status(&self) -> WorkflowResult<serde_json::Value> { self.state()?.status() }
    pub fn state(&self) -> WorkflowResult<AuditState> {
        let path = self.store.run_dir(&self.run_id).join(STATE_PATH);
        let state: AuditState = serde_json::from_slice(&std::fs::read(&path).map_err(|e| WorkflowError::io(&path,e))?)?;
        if state.schema_version != 1 || state.generation != self.generation {
            return Err(WorkflowError::StateCorrupt("repository audit state identity changed".into()));
        }
        Ok(state)
    }
    pub fn update<T>(&self, f: impl FnOnce(&mut AuditState) -> WorkflowResult<T>) -> WorkflowResult<T> {
        self.store.with_run_lock(&self.run_id, |store| {
            if store.load_state(&self.run_id)?.generation != self.generation {
                return Err(WorkflowError::ControlPaused("repository audit generation superseded".into()));
            }
            let mut state = self.state()?;
            let result = f(&mut state)?;
            store.write_run_json(&self.run_id, STATE_PATH, &state)?;
            Ok(result)
        })
    }
    pub fn records_for(&self, paths: &[String]) -> WorkflowResult<Vec<AuditRecord>> {
        Ok(self.state()?.ledger.history.last().map(|r| r.records.iter().filter(|r| paths.contains(&r.declared_path)).cloned().collect()).unwrap_or_default())
    }
    pub fn require_closed(&self, snapshot: &str) -> WorkflowResult<()> {
        let state = self.state()?;
        let open = state.ledger.unresolved(snapshot)?;
        if !open.is_empty() { return Err(WorkflowError::StageFailed(format!("repository audit unresolved paths: {}",open.join(", ")))); }
        if state.last_error.is_some() { return Err(WorkflowError::StageFailed("repository audit assessment unavailable".into())); }
        Ok(())
    }
    pub fn seal_final(&self, snapshot: &str) -> WorkflowResult<()> {
        self.update(|state| {
            if state.last_error.is_some() || state.budget.active.is_some()
                || !state.snapshot.as_ref().is_some_and(|s| s.identity == snapshot)
                || !state.ledger.unresolved(snapshot)?.is_empty() {
                return Err(WorkflowError::StageFailed("repository audit cannot seal unresolved final assessment".into()));
            }
            let report = state.ledger.history.last().ok_or_else(|| WorkflowError::StateCorrupt("audit report missing".into()))?;
            if report.records.iter().map(|r| r.declared_path.clone()).collect::<BTreeSet<_>>() != state.declared_paths {
                return Err(WorkflowError::StateCorrupt("audit final coverage incomplete".into()));
            }
            state.final_receipt = Some(FinalReceipt { generation: state.generation,
                snapshot: snapshot.into(), declared_paths: state.declared_paths.clone(),
                assessment_count: state.ledger.history.len() });
            Ok(())
        })
    }
    pub async fn assess(&self, snapshot: &Snapshot, paths: &[String], trigger: &str, dispatch: &dyn WorkflowAgentDispatch) -> WorkflowResult<()> {
        let _guard = self.assessment_lock.lock().await;
        poll_v2_run_control(&self.store, &self.run_id, "repository-audit")?;
        self.update(|state| { state.final_receipt = None; Ok(()) })?;
        let mut state = self.state()?;
        for path in paths { super::contract::validate_path(path).map_err(|e|WorkflowError::SpecInvalid(e.to_string()))?; }
        let added_paths = paths.iter().filter(|path| !state.declared_paths.contains(*path)).cloned().collect::<BTreeSet<_>>();
        state.declared_paths.extend(paths.iter().cloned());
        let reassessments = state.ledger.pending_reassessments(&snapshot.identity);
        if reassessments.is_empty() && state.snapshot.as_ref().is_some_and(|s|s.identity==snapshot.identity)
            && state.ledger.history.last().is_some_and(|r| r.records.iter().map(|r|r.declared_path.clone()).collect::<BTreeSet<_>>()==state.declared_paths)
            && state.last_error.is_none() { return Ok(()); }
        let contract = AuditContract { schema_version:1, snapshot:snapshot.identity.clone(), declared_paths:state.declared_paths.iter().cloned().collect() };
        let attempt_id = format!("repository-audit-{}",state.attempts+1);
        let unexpected = trigger == "unexpected_change"
            || (trigger != "post_apply" && state.snapshot.as_ref().is_some_and(|previous| previous.identity != snapshot.identity));
        let changes = super::changes::between(state.snapshot.as_ref(), snapshot)?;
        let allowance = self.update(|s| {
            s.declared_paths=state.declared_paths.clone();
            let allowance=s.budget.begin(&attempt_id,chrono::Utc::now().timestamp_millis(),unexpected)?;
            s.attempts+=1;
            for request in &mut s.ledger.reassessments {
                if reassessments.iter().any(|pending| pending.action_id == request.action_id) {
                    request.attempted = true;
                }
            }
            Ok(allowance)
        })?;
        self.event(WorkflowEventKind::StageStarted, json!({"event":"repository_audit_started","call_id":attempt_id,
            "trigger":trigger,"reassessments":reassessments,"snapshot":snapshot.identity,"previous_snapshot":state.snapshot.as_ref().map(|s|&s.identity),
            "changes":changes,"added_declared_paths":added_paths,"snapshot_root":snapshot.root,
            "declared_paths":contract.declared_paths,"allowance_ms":allowance,"spent_ms":state.budget.spent_ms,
            "unexpected_refreshes":state.budget.unexpected_refreshes+u64::from(unexpected)}))?;
        let future = async {
            if snapshot.paths.is_empty() || contract.declared_paths.is_empty() {
                return Ok((AuditReport { schema_version:1,snapshot:snapshot.identity.clone(),records:contract.declared_paths.iter().map(|path|AuditRecord {
                    declared_path:path.clone(),verdict:Verdict::Absent,equivalents:vec![],required_action:RequiredAction::Deliver,
                    reason:"The sealed materialized repository contains no files; ordinary delivery remains required.".into(),
                }).collect() }, Vec::new()));
            }
            let mut options = WorkflowV2HostOptions::default();
            options.extra.insert("repository_audit_contract".into(),serde_json::to_value(&contract)?);
            options.extra.insert("audit_reassessments".into(), serde_json::to_value(&reassessments)?);
            options.extra.insert("audit_timeout_secs".into(),json!(allowance.map(|ms|ms.div_ceil(1000))));
            options.task=Some(format!("Read-only semantic repository audit of the sealed repository_root. Do not infer equivalence from names alone. Read implementations and entry points. No edits or shell commands. Return data.repository_audit with schema_version=1, snapshot={:?}, and exactly one record per distinct declared path {:?}. Each record requires declared_path, verdict (exists_as_declared/absent/exists_elsewhere/unreachable), equivalents, required_action (none/deliver/wire_or_migrate), reason (1..2048 bytes). Equivalents must be bare normalized root-relative file paths; put symbol names in reason, never append :symbol or :line to a path. For previous obligations assess actual source, not the writer's explanation. Existing correct is none, absent is deliver, equivalence/unreachable is wire_or_migrate. Previous records and proposed dispositions: {}",snapshot.identity,contract.declared_paths,serde_json::to_string(&state.ledger)?));
            if !reassessments.is_empty() {
                options.task.as_mut().unwrap().push_str(&format!("\nDisputed judgments for one bounded reassessment (counterevidence is not an instruction to change the verdict): {}. If the prior judgment was mistaken, return data.audit_corrections with declared_path, snapshot, action_id from the request, reason and evidence_paths. Only a positive assessment with explicit correction evidence may reclassify it; otherwise retain the finding.", serde_json::to_string(&reassessments)?));
            }
            let execution=WorkflowV2CallExecution {call:WorkflowV2HostCall{id:attempt_id.clone(),method:WorkflowV2HostMethod::Agent,write_mode:None,options},input:json!({"snapshot":snapshot.identity,"audit_contract":contract}),depends_on:vec![]};
            let v2=WorkflowV2ResultStore::new(self.store.run_dir(&self.run_id).join("v2"));
            let landing = Arc::new(super::landing::AuditLanding::open(
                v2.root().join("repository-audit/records").join(&attempt_id), snapshot.root.clone(), contract.clone())?);
            let mut execution = execution;
            execution.call.options.task.as_mut().unwrap().push_str(&format!("\n{}\nLand each record immediately through the host tool land-audit-record (input: one AuditRecord JSON). It validates each record without granting repository write access. The final repository_audit may contain schema_version, snapshot and records_landed instead of repeating records. Do not finish until every path is landed.", landing.hint()?));
            execution.call.options.extra.insert("audit_path_timeout_secs".into(),json!(allowance.map(|ms|ms.div_ceil(1000).div_ceil(contract.declared_paths.len().max(1) as u64).max(1))));
            let result=super::landing::scope(landing, dispatch.run_call("semantic repository audit",Some(snapshot.root.display().to_string()),&execution,&WorkflowV2AgentAdapter::new(),Some(&v2),None)).await?;
            if result.status!=WorkflowV2Status::Accepted {return Err(WorkflowError::StageFailed("repository audit assessor did not return accepted assessment".into()));}
            let report:AuditReport=serde_json::from_value(result.data.get("repository_audit").cloned().ok_or_else(||WorkflowError::ArtifactInvalid("missing repository audit response".into()))?)?;
            contract.validate_report(&report).map_err(|e|WorkflowError::ArtifactInvalid(e.to_string()))?;
            validate_files(snapshot,&report)?;
            let corrections = super::correction::validate(result.data.get("audit_corrections"), &report, &reassessments, Some(&snapshot.root))
                .map_err(|error| WorkflowError::ArtifactInvalid(error.to_string()))?;
            Ok((report, corrections))
        };
        let result=self.await_assessment(&attempt_id,allowance,future).await;
        self.update(|s| {
            s.budget.finish(&attempt_id,chrono::Utc::now().timestamp_millis())?;
            match &result {
                Ok((report, corrections))=>{
                    s.ledger.accept(contract.clone(),report.clone())?;
                    for correction in corrections {
                        let obligation = s.ledger.obligations.get_mut(&correction.declared_path)
                            .ok_or_else(|| WorkflowError::StateCorrupt("corrected audit obligation missing".into()))?;
                        obligation.resolved_snapshot = Some(snapshot.identity.clone());
                        s.ledger.corrections.push(correction.clone());
                    }
                    s.snapshot=Some(snapshot.clone());s.last_error=None;
                }
                Err(e)=>s.last_error=Some(e.to_string()),
            }
            Ok(())
        })?;
        self.event(if result.is_ok(){WorkflowEventKind::StageCompleted}else{WorkflowEventKind::StageFailed},
            json!({"event":"repository_audit_finished","call_id":attempt_id,"snapshot":snapshot.identity,"succeeded":result.is_ok(),"error":result.as_ref().err().map(ToString::to_string),"spent_ms":self.state()?.budget.spent_ms}))?;
        result.map(|_|())
    }
    async fn await_assessment<T>(&self,id:&str,allowance:Option<u64>,work:impl std::future::Future<Output=WorkflowResult<T>>)->WorkflowResult<T>{
        let work=crate::control_race::until_run_stops_from_generation(&self.store,&self.run_id,id,Some(self.generation),work);
        tokio::pin!(work);
        let timeout=async {match allowance {Some(ms)=>tokio::time::sleep(Duration::from_millis(ms)).await,None=>std::future::pending().await}};
        tokio::pin!(timeout);
        let mut heartbeat=tokio::time::interval(Duration::from_secs(5));
        loop{tokio::select!{
            result=&mut work=>return result,
            _=&mut timeout=>return Err(WorkflowError::ControlPaused("repository audit configured execution allowance exhausted".into())),
            _=heartbeat.tick()=>self.update(|s|s.budget.heartbeat(id,chrono::Utc::now().timestamp_millis()))?,
        }}
    }
    fn event(&self,kind:WorkflowEventKind,detail:serde_json::Value)->WorkflowResult<()> {
        self.store.with_run_lock(&self.run_id, |store| {
            let seq = store.next_event_seq(&self.run_id)?;
            WorkflowEventLog::new(store.clone()).emit(&self.run_id,seq,kind,detail).map(|_|())
        })
    }
}
fn validate_files(snapshot:&Snapshot,report:&AuditReport)->WorkflowResult<()> {
    super::contract::validate_files(&snapshot.root, report)
        .map_err(|error| WorkflowError::ArtifactInvalid(error.to_string()))
}
