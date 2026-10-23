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
//! entries and their judgments are untouched. A repair the author or judge
//! cannot complete becomes a contract defect: failing, blocking, owned by no
//! task, and carrying the operator command that repairs it. A host-unproven
//! check remains pending with an operational error so the round retries it.
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
    AUTHORED_PENDING_PREFIX, AcceptanceCheckRecordV1, AcceptanceCheckStatus,
    AcceptanceContractRepairV1, AcceptanceRoundRecordV1, REPAIR_TRIGGER_NOT_ACCEPTED,
    REPAIR_TRIGGER_SCRIPT_DEFECT,
};
use archon_workflow::{WorkflowError, WorkflowLlmClient, WorkflowResult, WorkflowStore};

use super::exec::{self, StageContext};
use crate::command::workflow_task_set::executability::{
    Baseline, FailedTree, HOST_UNPROVEN, HostProbe, crash_findings_at, originals,
};
use crate::command::workflow_task_set::reauthor::{AuthorScope, ReauthorGate};
use crate::command::workflow_task_set::republish::{
    ReauthorRequest, ReauthorResult, reauthor_and_republish, reauthor_command,
};
use crate::command::workflow_task_set::{non_accepted_ids, pause_if_unsettled};

pub(super) const HOST_PENDING_PREFIX: &str = "host pending:";

pub(super) fn repair_author_scope(
    project: &Path,
    task_root: &Path,
    repository: &Path,
    prd_path: &Path,
    base: Option<&str>,
) -> anyhow::Result<AuthorScope> {
    let mut scope =
        AuthorScope::for_task_set(project, task_root, prd_path).map_err(anyhow::Error::msg)?;
    scope.repository_root = repository.to_path_buf();
    if let Some(commit) = base {
        scope.baseline = Some(Baseline {
            commit: commit.to_string(),
            repository: repository.to_path_buf(),
        });
    }
    Ok(scope)
}

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
    let probe = HostProbe::at_with_limits(
        context.project.clone(),
        context.repository.clone(),
        context.binding.clone(),
        context.cargo_limits.clone(),
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
    let scope = repair_author_scope(
        &context.project,
        &context.task_root,
        &context.repository,
        &prd_path,
        base,
    )?;
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
        &scope,
    )
    .await
}

#[path = "workflow_live_v3_acceptance_repair_unaccepted.rs"]
mod unaccepted;
pub(super) use unaccepted::apply;
#[cfg(all(test, unix))]
pub(super) use unaccepted::repair_unaccepted;

/// The round's site and baseline for re-running a repaired check.
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

/// Repair, republish and re-run every check named in `defects`.
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
    // The round's own site is the tree these checks just ran on, and they
    // are judged there, never on the host.
    let site = HostProbe::at_with_limits(
        round.context.project.clone(),
        round.context.repository.clone(),
        round.context.binding.clone(),
        round.context.cargo_limits.clone(),
    )
    .check_site(contract);
    let failed = FailedTree {
        commit: None,
        originals: originals(
            &site,
            contract,
            ids.iter()
                .filter_map(|id| results.get(id))
                .cloned()
                .collect(),
        )
        .await,
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
    // Issue 336: a republished set whose interrupted publish cannot be
    // settled pauses the run; it is never this repair's failure, nor is one
    // the republish itself met (Issue 338).
    if let Some(paused) = outcome.as_ref().err().and_then(pause_if_unsettled) {
        return Err(paused);
    }
    let reloaded = match outcome.map(|result| (result, exec::load_contract(round.context))) {
        Ok((_, Err(paused @ WorkflowError::ControlPaused(_)))) => return Err(paused),
        Ok((result, loaded)) => loaded
            .map(|(reloaded, digest, _)| (result, reloaded, digest))
            .map_err(|error| anyhow!("the republished contract is not usable: {error}")),
        Err(error) => Err(error),
    };
    let (result, reloaded, digest) = match reloaded {
        Ok(reloaded) => reloaded,
        Err(error) => {
            // The re-author/probe path can return the typed incomplete reason
            // rendered with HOST_UNPROVEN; keep this routed as host-owned.
            if format!("{error:#}").contains(HOST_UNPROVEN) {
                record.operational_errors.push(format!("{error:#}"));
                record.contract_repairs.push(AcceptanceContractRepairV1 {
                    check_ids: ids.iter().cloned().collect(),
                    trigger: trigger.into(),
                    repaired: false,
                    freeze_event_id: String::new(),
                    failure: format!("{HOST_PENDING_PREFIX}{error:#}"),
                    diagnostics: Vec::new(),
                });
                return Ok(Defects::new());
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
    let unresolved = result.unproven.clone();
    let authored_pending: BTreeSet<_> = result.authored_pending.keys().cloned().collect();
    let published_ids: Vec<_> = ids
        .iter()
        .filter(|id| !unresolved.contains_key(*id))
        .cloned()
        .collect();
    if !published_ids.is_empty() {
        record.contract_repairs.push(AcceptanceContractRepairV1 {
            check_ids: published_ids,
            trigger: trigger.into(),
            repaired: result.published,
            freeze_event_id: result.freeze_event_id.clone(),
            failure: String::new(),
            diagnostics: result.diagnostics.clone(),
        });
    }
    for (pending, prefix) in [
        (true, AUTHORED_PENDING_PREFIX),
        (false, HOST_PENDING_PREFIX),
    ] {
        let group: Vec<_> = unresolved
            .iter()
            .filter(|(id, _)| authored_pending.contains(*id) == pending)
            .collect();
        if group.is_empty() {
            continue;
        }
        if !pending {
            record.operational_errors.extend(
                group
                    .iter()
                    .map(|(id, reason)| format!("{HOST_UNPROVEN} {id}: {reason}")),
            );
        }
        let details = group
            .iter()
            .map(|(id, reason)| format!("{id}: {reason}"))
            .collect::<Vec<_>>()
            .join(" | ");
        let remedy = if trigger == REPAIR_TRIGGER_SCRIPT_DEFECT {
            format!("; operator re-author command: {command}")
        } else {
            String::new()
        };
        let failure = format!("{prefix}{details}{remedy}");
        record.contract_repairs.push(AcceptanceContractRepairV1 {
            check_ids: group.iter().map(|(id, _)| (*id).clone()).collect(),
            trigger: trigger.into(),
            repaired: false,
            freeze_event_id: result.freeze_event_id.clone(),
            failure,
            diagnostics: result.diagnostics.clone(),
        });
    }
    (*contract, *chain_digest) = (reloaded, digest);
    let selected: Vec<&AcceptanceCriterion> = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .filter(|criterion| ids.contains(&criterion.id) && !unresolved.contains_key(&criterion.id))
        .collect();
    if selected.is_empty() {
        return Ok(Defects::new());
    }
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
#[cfg(all(test, unix))]
#[path = "workflow_live_v3_acceptance_repair_pause_tests.rs"]
mod pause_tests;
#[cfg(all(test, unix))]
#[path = "workflow_live_v3_acceptance_repair_provider_tests.rs"]
mod provider_tests;
