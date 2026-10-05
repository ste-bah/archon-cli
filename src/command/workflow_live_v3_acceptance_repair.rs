//! In-round repair of frozen checks no task can fix: checks the judge did
//! not accept, and checks that crashed in their own code.
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
//! A check that ran this round and crashed in its own code
//! (`acceptance_check_crash`) is the same kind of defect: its crash is fed to
//! the same bounded re-author, the repair must be re-accepted by the judge
//! and must itself run without crashing (the executability gate), the chain
//! is republished, and the repaired check runs again in this round. It is
//! never handed to the implementing tasks; if the repair fails it is a
//! contract defect like an unaccepted one.
//!
//! Batch O (A5): a repaired check must also be able to FAIL. The repair's
//! executability gate runs it on the run's base commit -- the tree before
//! any implementation, in a hermetic copy -- and a repair that passes there
//! goes back to its author like a crash (`executability::Baseline`); a
//! repaired check is never weaker than "fails where nothing was built". It
//! is also run on the tree the check it replaces ran on this round (the
//! round's own site), and held to that check's verdict there
//! (`executability::FailedTree`): where the original failed its own
//! assertion the repair must fail too, since a repair may fix how a check
//! asserts, never turn a failing product green; where the original's failure
//! was the check's own defect (a crash, a check the host could not run, a
//! contract defect) it gave no verdict there, and the repair's is the first.
//!
//! The chain the round then verifies is the republished one: `load_contract`
//! and the scratch guardian both re-read the current pin from disk. The
//! republish records a lineage link and files the chain it replaced, so the
//! round and the run-end observer prove the pin reached from the launch pin.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::{AcceptanceContract, AcceptanceCriterion};
use archon_workflow::v2::acceptance_stage::{
    AcceptanceCheckRecordV1, AcceptanceCheckStatus, AcceptanceContractRepairV1,
    AcceptanceRoundRecordV1, REPAIR_TRIGGER_NOT_ACCEPTED, REPAIR_TRIGGER_SCRIPT_DEFECT,
};
use archon_workflow::{WorkflowLlmClient, WorkflowResult, WorkflowStore};

use super::exec::{self, StageContext};
use crate::command::workflow_task_set::executability::{
    Baseline, FailedTree, HOST_UNPROVEN, HostProbe, crash_findings_at, originals,
};
use crate::command::workflow_task_set::non_accepted_ids;
use crate::command::workflow_task_set::reauthor::{AuthorScope, ReauthorGate};
use crate::command::workflow_task_set::republish::{
    ReauthorRequest, ReauthorResult, reauthor_and_republish, reauthor_command,
};

/// Contract-defect text per check id the repair could not fix.
pub(super) type Defects = BTreeMap<String, String>;

/// Re-author, re-judge, probe and republish exactly `ids`, at the round's
/// own execution site.
pub(super) async fn republish(
    llm: Option<&dyn WorkflowLlmClient>,
    context: &StageContext,
    contract: &AcceptanceContract,
    ids: &BTreeSet<String>,
    seeds: &BTreeMap<String, String>,
    base: Option<&str>,
    failed: Option<FailedTree>,
) -> anyhow::Result<ReauthorResult> {
    let Some(llm) = llm else {
        return Err(anyhow!(
            "this acceptance stage has no author client to re-author with"
        ));
    };
    let prd_path = {
        let path = PathBuf::from(&contract.prd.path);
        if path.is_absolute() {
            path
        } else {
            context.project.join(path)
        }
    };
    let probe = HostProbe::at(
        context.project.clone(),
        context.repository.clone(),
        context.binding.clone(),
    );
    let probe = match base {
        Some(commit) => probe.with_baseline(Baseline {
            commit: commit.to_string(),
            repository: context.repository.clone(),
        }),
        None => probe,
    };
    let probe = match failed {
        Some(tree) => probe.with_failed_tree(tree),
        None => probe,
    };
    // Each gate republishes under the mode its stage was frozen in.
    reauthor_and_republish(
        llm,
        ReauthorRequest {
            project_root: &context.project,
            tasks_root: &context.task_root,
            prd_path: &prd_path,
            ids,
            gate: ReauthorGate {
                probe: &probe,
                seeds,
            },
            trigger: "in-round acceptance repair",
        },
        &AuthorScope {
            prd_path: prd_path.clone(),
            project_root: context.project.clone(),
            repository_root: context.repository.clone(),
        },
    )
    .await
}

/// Repair every non-accepted check in `contract`. `None` when there is none.
pub(super) async fn repair_unaccepted(
    llm: Option<&dyn WorkflowLlmClient>,
    context: &StageContext,
    contract: &AcceptanceContract,
    base: Option<&str>,
) -> Option<(AcceptanceContractRepairV1, Defects)> {
    let ids = non_accepted_ids(contract);
    if ids.is_empty() {
        return None;
    }
    // A check the judge never accepted never ran: there is no verdict of
    // its own to hold a repair to, only the baseline.
    let outcome = republish(llm, context, contract, &ids, &BTreeMap::new(), base, None).await;
    let check_ids = ids.iter().cloned().collect::<Vec<_>>();
    Some(match outcome {
        Ok(result) => (
            AcceptanceContractRepairV1 {
                check_ids,
                trigger: REPAIR_TRIGGER_NOT_ACCEPTED.into(),
                repaired: true,
                freeze_event_id: result.freeze_event_id,
                failure: String::new(),
                diagnostics: result.diagnostics,
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
                    trigger: REPAIR_TRIGGER_NOT_ACCEPTED.into(),
                    repaired: false,
                    freeze_event_id: String::new(),
                    failure: format!("{error:#}"),
                    diagnostics: Vec::new(),
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
    base: Option<&str>,
) -> Option<Defects> {
    let Some((repair, defects)) = repair_unaccepted(llm, context, contract, base).await else {
        return Some(Defects::new());
    };
    let repaired = repair.repaired;
    // A repair the host could not prove is the host's: the round records it
    // as an operational error, and the next round tries again.
    if !repaired && repair.failure.contains(HOST_UNPROVEN) {
        record.operational_errors.push(repair.failure.clone());
    }
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

/// Where the round ran its checks, for re-running a repaired one there.
pub(super) struct Round<'a> {
    pub(super) llm: Option<&'a dyn WorkflowLlmClient>,
    pub(super) context: &'a StageContext,
    pub(super) store: &'a WorkflowStore,
    pub(super) run_id: &'a str,
    pub(super) call_id: &'a str,
    pub(super) evidence_dir: &'a Path,
    /// The run's base commit: the tree every repaired check must fail on.
    pub(super) base: Option<&'a str>,
}

/// Checks that ran this round and go back to their author: each one's
/// finding, the repair's trigger, where their evidence is kept, and why no
/// task can fix them.
pub(super) struct RanDefects<'a> {
    pub(super) findings: BTreeMap<String, String>,
    pub(super) trigger: &'a str,
    pub(super) evidence: &'a str,
    pub(super) why: &'a str,
}

/// Repair, republish and re-run in this round every check in `results` that
/// crashed in its own code. Repaired checks' re-run results replace the
/// crashed ones; a check still crashing, or one the repair could not fix, is
/// returned as a contract defect (its result stays in `results` as the
/// defect's evidence). `contract` and `chain_digest` become the republished
/// chain.
pub(super) async fn repair_crashed(
    round: &Round<'_>,
    contract: &mut AcceptanceContract,
    chain_digest: &mut String,
    results: &mut BTreeMap<String, CheckResult>,
    record: &mut AcceptanceRoundRecordV1,
) -> WorkflowResult<Defects> {
    let crashed = crash_findings_at(contract, results.values(), round.context.binding.as_ref());
    let defects = RanDefects {
        findings: crashed,
        trigger: REPAIR_TRIGGER_SCRIPT_DEFECT,
        evidence: "script-defect",
        why: "No task can fix a check that never asserts its criterion",
    };
    repair_ran(round, contract, chain_digest, results, record, defects).await
}

/// Repair, republish and re-run in this round every check `defects` names
/// (see [`repair_crashed`]); a repaired check that crashes on its re-run is
/// a contract defect too.
pub(super) async fn repair_ran(
    round: &Round<'_>,
    contract: &mut AcceptanceContract,
    chain_digest: &mut String,
    results: &mut BTreeMap<String, CheckResult>,
    record: &mut AcceptanceRoundRecordV1,
    defects: RanDefects<'_>,
) -> WorkflowResult<Defects> {
    let RanDefects {
        findings: crashed,
        trigger,
        evidence,
        why,
    } = defects;
    if crashed.is_empty() {
        return Ok(Defects::new());
    }
    // The defect itself is kept as evidence beside the round's outputs.
    for id in crashed.keys() {
        if let Some(result) = results.get(id) {
            super::write_output_files(&round.evidence_dir.join(evidence), result);
        }
    }
    let ids: BTreeSet<String> = crashed.keys().cloned().collect();
    let command = reauthor_command(
        &round.context.project,
        &round.context.task_root,
        contract,
        &ids,
    );
    // The round's own site is the tree these checks just ran on.
    let failed = FailedTree {
        commit: None,
        originals: originals(contract, ids.iter().filter_map(|id| results.get(id))),
    };
    let outcome = republish(
        round.llm,
        round.context,
        contract,
        &ids,
        &crashed,
        round.base,
        Some(failed),
    )
    .await;
    let reloaded = outcome.and_then(|result| {
        exec::load_contract(round.context)
            .map(|(reloaded, digest, _)| (result, reloaded, digest))
            .map_err(|error| anyhow!("the republished contract is not usable: {error}"))
    });
    let (result, reloaded, digest) = match reloaded {
        Ok(reloaded) => reloaded,
        Err(error) => {
            if format!("{error:#}").contains(HOST_UNPROVEN) {
                record.operational_errors.push(format!("{error:#}"));
            }
            record.contract_repairs.push(AcceptanceContractRepairV1 {
                check_ids: ids.iter().cloned().collect(),
                trigger: trigger.into(),
                repaired: false,
                freeze_event_id: String::new(),
                failure: format!("{error:#}"),
                diagnostics: Vec::new(),
            });
            // The crashed results stay in `results`: the defect records
            // carry their exit code and output.
            return Ok(crashed
                .iter()
                .map(|(id, finding)| {
                    (
                        id.clone(),
                        format!(
                            "contract defect: {finding}\n{why}; the in-round re-author did not produce an accepted check that runs ({error:#}); repair the contract with: {command}"
                        ),
                    )
                })
                .collect());
        }
    };
    record.contract_repairs.push(AcceptanceContractRepairV1 {
        check_ids: ids.iter().cloned().collect(),
        trigger: trigger.into(),
        repaired: true,
        freeze_event_id: result.freeze_event_id,
        failure: String::new(),
        diagnostics: result.diagnostics,
    });
    (*contract, *chain_digest) = (reloaded, digest);
    let selected: Vec<&AcceptanceCriterion> = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .filter(|criterion| ids.contains(&criterion.id))
        .collect();
    let rerun = exec::checks::execute_checks(
        round.store,
        round.run_id,
        round.call_id,
        round.context,
        contract,
        chain_digest,
        &selected,
        &round.evidence_dir.join("repaired"),
    )
    .await?;
    // Batch G: a site failure on the re-run is the round's, not the checks'.
    if !rerun.site_errors.is_empty() {
        record.operational_errors.extend(rerun.site_errors);
        // Their crash ran under the old contract: no record for them.
        ids.iter().for_each(|id| drop(results.remove(id)));
        return Ok(Defects::new());
    }
    let rerun = rerun.results;
    let still = crash_findings_at(contract, &rerun, round.context.binding.as_ref());
    let mut defects = Defects::new();
    for result in rerun {
        if let Some(finding) = still.get(&result.acceptance_id) {
            defects.insert(
                result.acceptance_id.clone(),
                format!(
                    "contract defect: the repaired check crashed when this round re-ran it: {finding}\nNo task can fix a check that never asserts its criterion; repair the contract with: {command}"
                ),
            );
        }
        // The re-run is the round's result for the check, defect or not.
        results.insert(result.acceptance_id.clone(), result);
    }
    Ok(defects)
}

/// The record of a check that is a contract defect: failing, never owned.
/// `result` is the run that showed the defect, when the check ran at all.
pub(super) fn defect_record(
    criterion: &AcceptanceCriterion,
    defect: &str,
    result: Option<&CheckResult>,
) -> AcceptanceCheckRecordV1 {
    AcceptanceCheckRecordV1 {
        check_id: criterion.id.clone(),
        criterion: criterion.criterion.clone(),
        kind: exec::check_kind(criterion).to_string(),
        status: AcceptanceCheckStatus::Error,
        exit_code: result.and_then(|result| result.exit_code),
        operational_error: Some(defect.to_string()),
        owning_tasks: Vec::new(),
        stdout_tail: result.map_or_else(String::new, |result| super::output::tail(&result.stdout)),
        stderr_tail: result.map_or_else(String::new, |result| super::output::tail(&result.stderr)),
        // Never attributed to a landing either: no task's change broke it.
        regressed_by: None,
        contract_defect: true,
        routing: None,
        regression_search: None,
        blocked: None,
    }
}

#[cfg(all(test, unix))]
#[path = "workflow_live_v3_acceptance_repair_base_tests.rs"]
mod base_tests;
