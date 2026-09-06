//! Observation transaction: resolve, snapshot, execute, clean up, audit.
use super::*;
use std::sync::{Arc,atomic::{AtomicBool}};
use crate::acceptance_world::{FrozenCommandRef,resolve_command};
use crate::task_set_contract::{AcceptanceContract,content_digest};
use super::process::{CheckResult,run};

#[derive(Debug,Serialize,Deserialize)]
pub struct ObservationResult {
    pub checks:Vec<CheckResult>,
    pub live_roots_unchanged:bool,
    pub teardown_verified:bool,
    pub source_commit:String,
    pub policy_digest:String,
    pub before:BTreeMap<String,BTreeMap<String,String>>,
    pub after:BTreeMap<String,BTreeMap<String,String>>,
}
impl ObservationResult {
    pub fn passed(&self)->bool {
        self.live_roots_unchanged && self.teardown_verified && !self.checks.is_empty()
            && self.checks.iter().all(|c|c.exit_code==Some(0)&&c.operational_error.is_none())
    }
}
fn live(policy:&ScratchPolicy)->WorkflowResult<BTreeMap<String,BTreeMap<String,String>>> {
    [&policy.repository,&policy.project,&policy.task_root].into_iter()
        .map(|p|Ok((p.display().to_string(),inventory(p)?))).collect()
}
/// The caller supplies an integrity-validated pinned contract and chain digest.
/// Authorization is repeated here before creating scratch or spawning children.
pub async fn observe_commands(
    policy:&ScratchPolicy,commit:&str,contract:&AcceptanceContract,chain_digest:&str,
    refs:&[FrozenCommandRef],evidence:&Path,
)->WorkflowResult<ObservationResult> {
    observe_commands_cancellable(policy,commit,contract,chain_digest,refs,evidence,Arc::new(AtomicBool::new(false))).await
}
pub async fn observe_commands_cancellable(
    policy:&ScratchPolicy,commit:&str,contract:&AcceptanceContract,chain_digest:&str,
    refs:&[FrozenCommandRef],evidence:&Path,cancel:Arc<AtomicBool>,
)->WorkflowResult<ObservationResult> {
    policy.validate()?;
    let commands=refs.iter().map(|r|resolve_command(contract,chain_digest,r)).collect::<WorkflowResult<Vec<_>>>()?;
    if commands.is_empty() {return Err(invalid("no frozen commands selected for native observation"));}
    for root in [&policy.repository,&policy.project,&policy.task_root] {
        if evidence.starts_with(root) {return Err(invalid("provisional evidence must be outside live roots"));}
    }
    let before=live(policy)?;
    let mut roots=ScratchRoots::prepare(policy,commit)?;
    let mut checks=Vec::new();
    for (reference,command) in refs.iter().zip(commands) {
        if reference.kind == crate::acceptance_world::AcceptanceCommandKind::NestedVerifier {
            let entry = contract.acceptance.iter().chain(&contract.supplementary)
                .find(|entry|entry.id==reference.acceptance_id).expect("authorized entry");
            if let crate::task_set_contract::AcceptanceCheck::Floor { contract: floor } = &entry.check {
                let mut floor = floor.clone();
                floor.typed_verifier_command = None;
                let facts = crate::collect_declarative_floor_facts(roots.project(), &floor)?;
                match crate::evaluate_declarative_floor(&floor, &facts) {
                    crate::DeclarativeFloorEvaluation::Passed => {},
                    crate::DeclarativeFloorEvaluation::Failed { findings } => {
                        checks.push(CheckResult {acceptance_id:reference.acceptance_id.clone(),exit_code:Some(1),stdout:vec![],stderr:findings.join("; ").into_bytes(),operational_error:None});
                        continue;
                    },
                    crate::DeclarativeFloorEvaluation::Deferred { reason } => {
                        checks.push(CheckResult {acceptance_id:reference.acceptance_id.clone(),exit_code:None,stdout:vec![],stderr:vec![],operational_error:Some(reason)});
                        continue;
                    },
                }
            }
        }
        match run(&roots,policy,&reference.acceptance_id,&command,cancel.clone()).await {
            Ok(result)=>{let stop=result.operational_error.is_some();checks.push(result);if stop {break;}},
            Err(error)=>{checks.push(CheckResult {acceptance_id:reference.acceptance_id.clone(),exit_code:None,stdout:vec![],stderr:vec![],operational_error:Some(error.to_string())});break;}
        }
    }
    let cleanup=roots.cleanup();
    let after=live(policy)?;
    // Git may create an empty administrative worktrees directory when it did
    // not exist before. No file/content change is exempted from the audit.
    let normalize=|maps:&BTreeMap<String,BTreeMap<String,String>>| {
        let mut maps=maps.clone();
        for entries in maps.values_mut() {
            if entries.get(".git/worktrees").is_some_and(|v|v=="directory")
                && !entries.keys().any(|p|p.starts_with(".git/worktrees/")) {entries.remove(".git/worktrees");}
        }
        maps
    };
    let result=ObservationResult {checks,live_roots_unchanged:normalize(&before)==normalize(&after),
        teardown_verified:cleanup.is_ok(),source_commit:commit.into(),
        policy_digest:content_digest(&serde_json::to_vec(policy)?),before,after};
    std::fs::create_dir_all(evidence).map_err(|e|WorkflowError::io(evidence,e))?;
    let path=evidence.join("observation.json");
    std::fs::write(&path,serde_json::to_vec_pretty(&result)?).map_err(|e|WorkflowError::io(&path,e))?;
    Ok(result)
}
