//! In-round repair of frozen checks the judge did not accept.
//!
//! A refuted check can never run (`acceptance_world::resolve_command` refuses
//! it), and routing it to the tasks that implement it only spends remediation
//! on a check no code can turn green. So a round that meets one repairs the
//! CONTRACT first: the same bounded re-author and re-judge as
//! `workflow freeze-acceptance --reauthor`, for exactly those checks,
//! republishing contract, lock, skeleton, skeleton lock and pin atomically.
//! The round then runs the repaired checks with all the others; accepted
//! entries and their judgments are untouched. Only when the repair fails does
//! a check become a contract defect: failing, blocking, owned by no task, and
//! carrying the operator command that repairs it.
//!
//! The chain the round then verifies is the republished one: `load_contract`
//! and the scratch guardian both re-read the current pin from disk. The
//! run-end observer compares against the pin captured at launch and so
//! records `observer_state=failed`; it is observe-only.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::anyhow;
use archon_workflow::WorkflowLlmClient;
use archon_workflow::task_set_contract::{AcceptanceContract, AcceptanceCriterion};
use archon_workflow::v2::acceptance_stage::{
    AcceptanceCheckRecordV1, AcceptanceCheckStatus, AcceptanceContractRepairV1,
    AcceptanceRoundRecordV1,
};

use super::exec::{self, StageContext};
use crate::command::workflow_task_set::non_accepted_ids;
use crate::command::workflow_task_set::reauthor::AuthorScope;
use crate::command::workflow_task_set::republish::{
    ReauthorRequest, reauthor_and_republish, reauthor_command,
};

/// Contract-defect text per check id the repair could not fix.
pub(super) type Defects = BTreeMap<String, String>;

/// Repair every non-accepted check in `contract`. `None` when there is none.
pub(super) async fn repair_unaccepted(
    llm: Option<&dyn WorkflowLlmClient>,
    context: &StageContext,
    contract: &AcceptanceContract,
) -> Option<(AcceptanceContractRepairV1, Defects)> {
    let ids = non_accepted_ids(contract);
    if ids.is_empty() {
        return None;
    }
    let prd_path = {
        let path = PathBuf::from(&contract.prd.path);
        if path.is_absolute() {
            path
        } else {
            context.project.join(path)
        }
    };
    let outcome = match llm {
        None => Err(anyhow!(
            "this acceptance stage has no author client to re-author with"
        )),
        // Each gate republishes under the mode its stage was frozen in.
        Some(llm) => {
            reauthor_and_republish(
                llm,
                ReauthorRequest {
                    project_root: &context.project,
                    tasks_root: &context.task_root,
                    prd_path: &prd_path,
                    ids: &ids,
                },
                &AuthorScope {
                    prd_path: prd_path.clone(),
                    project_root: context.project.clone(),
                    repository_root: context.repository.clone(),
                },
            )
            .await
        }
    };
    let check_ids = ids.iter().cloned().collect::<Vec<_>>();
    Some(match outcome {
        Ok(result) => (
            AcceptanceContractRepairV1 {
                check_ids,
                repaired: true,
                freeze_event_id: result.freeze_event_id,
                failure: String::new(),
            },
            Defects::new(),
        ),
        Err(error) => {
            let command = reauthor_command(&context.project, &context.task_root, contract, &ids);
            let defects = ids
                .iter()
                .map(|id| {
                    (
                        id.clone(),
                        format!(
                            "contract defect: frozen check '{id}' was not accepted by the judge, so it can never run and no task can fix it; the in-round re-author did not produce an accepted check ({error:#}); repair the contract with: {command}"
                        ),
                    )
                })
                .collect();
            (
                AcceptanceContractRepairV1 {
                    check_ids,
                    repaired: false,
                    freeze_event_id: String::new(),
                    failure: format!("{error:#}"),
                },
                defects,
            )
        }
    })
}

/// Repair the round's contract in place and record the attempt. Returns the
/// checks that remain contract defects, or `None` when the repaired contract
/// could not be reloaded (recorded as the round's operational error).
pub(super) async fn apply(
    llm: Option<&dyn WorkflowLlmClient>,
    context: &StageContext,
    contract: &mut AcceptanceContract,
    chain_digest: &mut String,
    record: &mut AcceptanceRoundRecordV1,
) -> Option<Defects> {
    let Some((repair, defects)) = repair_unaccepted(llm, context, contract).await else {
        return Some(Defects::new());
    };
    let repaired = repair.repaired;
    record.contract_repairs.push(repair);
    if repaired {
        match exec::load_contract(context) {
            Ok((reloaded, digest, _)) => (*contract, *chain_digest) = (reloaded, digest),
            Err(error) => {
                record.operational_errors.push(format!(
                    "repaired acceptance contract is not usable: {error}"
                ));
                return None;
            }
        }
    }
    Some(defects)
}

/// The record of a check that is a contract defect: failing, never owned.
pub(super) fn defect_record(
    criterion: &AcceptanceCriterion,
    defect: &str,
) -> AcceptanceCheckRecordV1 {
    AcceptanceCheckRecordV1 {
        check_id: criterion.id.clone(),
        criterion: criterion.criterion.clone(),
        kind: exec::check_kind(criterion).to_string(),
        status: AcceptanceCheckStatus::Error,
        exit_code: None,
        operational_error: Some(defect.to_string()),
        owning_tasks: Vec::new(),
        stdout_tail: String::new(),
        stderr_tail: String::new(),
        // Never attributed to a landing either: no task's change broke it.
        regressed_by: None,
        contract_defect: true,
    }
}
