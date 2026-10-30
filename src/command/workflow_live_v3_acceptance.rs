//! The authored run's acceptance stage: host side of `acceptance-contract-run`.
//!
//! Obs-32. The v3 script's `await acceptance()` asks the host to run the task set's frozen
//! `acceptance-contract.json` on the repository tree. The host resolves the execution site,
//! runs the whole contract each round, routes failures to implementing tasks, and writes an
//! append-only round record under `v2/acceptance/<round>/`. The script's `checkIds` are validated
//! but never narrow the run; only the finalizer reads the round record named by the call record.
//!
//! Every round, including resumed ones, re-executes because its evidence may describe an old tree.

use std::path::Path;

#[cfg(all(test, unix))]
use archon_workflow::WorkflowV2Status;
use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::AcceptanceCriterion;
use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::acceptance_stage::{
    ACCEPTANCE_ROUND_RECORD_SCHEMA_VERSION, AcceptanceRoundRecordV1, relative_record_path,
    round_dir,
};
#[cfg(test)]
use archon_workflow::v2::acceptance_stage::{AcceptanceCheckRecordV1, AcceptanceCheckStatus};
use archon_workflow::{
    WorkflowError, WorkflowResult, WorkflowStore, WorkflowV2CallExecution, WorkflowV2Result,
    poll_v2_run_control,
};

use super::WorkflowV2ScriptRuntime;
use crate::command::workflow_task_set::executability::Baseline;
#[path = "workflow_live_v3_acceptance_author.rs"]
mod author;
#[path = "workflow_live_v3_acceptance_author_drift.rs"]
mod author_drift;
#[path = "workflow_live_v3_acceptance_record.rs"]
mod check_rec;
#[path = "workflow_live_v3_acceptance_drift.rs"]
mod drift;
#[path = "workflow_live_v3_acceptance_env.rs"]
mod env;
#[path = "workflow_live_v3_acceptance_exec.rs"]
mod exec;
#[path = "workflow_live_v3_acceptance_failability.rs"]
mod failability;
#[path = "workflow_live_v3_acceptance_ledger.rs"]
mod ledger;
#[path = "workflow_live_v3_acceptance_output.rs"]
mod output;
#[path = "workflow_live_v3_acceptance_pending.rs"]
mod pending;
#[path = "workflow_live_v3_acceptance_regression.rs"]
mod regression;
#[path = "workflow_live_v3_acceptance_repair.rs"]
mod repair;
#[cfg(test)]
use repair::repair_author_scope;
#[path = "workflow_live_v3_acceptance_result.rs"]
mod result;
use check_rec::check_record;
#[path = "workflow_live_v3_acceptance_sources.rs"]
mod sources;
#[path = "workflow_live_v3_acceptance_stalled.rs"]
mod stalled;
#[cfg(test)]
use archon_workflow::v2::acceptance_stage::ACCEPTANCE_STAGE_TOOL;
use output::{with_frozen_identity, write_output_files, write_repairs};
use result::result_for;

pub(super) fn is_acceptance_stage_call(execution: &WorkflowV2CallExecution) -> bool {
    archon_workflow::v2::script::is_acceptance_stage_call(&execution.call)
}

#[path = "workflow_live_v3_acceptance_request.rs"]
mod request;
use request::{StageRequest, parse_request};

/// Run one acceptance round and record it for `owner`: the generation the host
/// dispatched it at, or the executor of a run end (Issue 316).
pub(super) async fn run_acceptance_stage(
    runtime: &WorkflowV2ScriptRuntime,
    execution: &WorkflowV2CallExecution,
    store: &WorkflowStore,
    run_id: &str,
    owner: archon_workflow::control_pause::PauseOwner,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    llm: Option<&dyn archon_workflow::WorkflowLlmClient>,
) -> WorkflowResult<WorkflowV2Result> {
    let request = parse_request(execution)?;
    let call_id = execution.call.id.clone();
    let run_dir = store.run_dir(run_id);
    let writer = archon_workflow::stage_write::StageWriter {
        store: store.clone(),
        run_id: run_id.to_string(),
        owner,
    };
    let reservation = request::reserve(&writer, &run_dir, request.round).await?;
    let attempt = reservation.attempt;
    let mut record = AcceptanceRoundRecordV1 {
        schema_version: ACCEPTANCE_ROUND_RECORD_SCHEMA_VERSION,
        run_id: run_id.to_string(),
        call_id: call_id.clone(),
        round: request.round,
        attempt,
        progress_frontier: Some(reservation.frontier),
        max_rounds: request.max_rounds,
        contract_present: false,
        requested_check_ids: request.check_ids.clone(),
        execution: None,
        checks: Vec::new(),
        operational_errors: Vec::new(),
        contract_repairs: Vec::new(),
        final_round: true,
    };
    // Batch L (L1): no round runs on a tree that holds a remediation its
    // verifier refused. Whatever cannot be taken back out holds the round.
    let refused = archon_workflow::stage_write::scope(writer.clone(), async {
        archon_workflow::stage_write::with_write(|| {
            WorkflowResult::Ok(
                archon_workflow::v2::script::refused_landings::revert_refused_landings(
                    &archon_workflow::WorkflowV2ResultStore::new(run_dir.join("v2")),
                    runtime.target_repository_root.as_deref().map(Path::new),
                ),
            )
        })
    })
    .await?;
    let mut frozen = None;
    let rounds = Box::pin(evaluate(
        runtime,
        store,
        run_id,
        &call_id,
        task_universe,
        llm,
        &request,
        &run_dir,
        &mut record,
        &mut frozen,
    ));
    // Batch G: what the round starts on the host is watched for input changes.
    let (evaluation, violation) = archon_workflow::write_coordinator::input_tripwire::watch_owned(
        writer.clone(),
        Some(&run_dir),
        &format!("acceptance round {} ({call_id})", request.round),
        archon_workflow::control_race::until_run_stops(
            store,
            run_id,
            &call_id,
            archon_workflow::stage_write::scope(writer, async {
                rounds.await?;
                archon_workflow::stage_write::with_write(|| WorkflowResult::Ok(()))
            }),
        ),
    )
    .await?;
    // A pause or cancel unwinds without a record: the round re-enters on
    // resume as the next attempt. Anything else is the round's own outcome.
    evaluation?;
    let violation = violation.map(|v| v.message());
    record.operational_errors.extend(violation);
    record.operational_errors.extend(refused.findings);
    // A2: the loop ends on a clean round or on nothing to act on; never on a
    // round count, an error the host can retry, or a check it can reassign.
    // Decision A: the failing states this run reached and the revisits in a
    // row, kept across attempts and resumes.
    // Issue 262 (round 8): damaged history heals or pauses, never fails.
    // Issue 320: without a task set, a context that will not resolve is final.
    let task_set = task_universe.is_some();
    let ledger::Decided { decision, path } = ledger::record_and_decide(
        store,
        run_id,
        owner,
        &run_dir,
        &mut record,
        task_set,
        Some(&reservation),
    )?;
    // Issue 262: pause on a stalled round with its record as evidence.
    if let Some(cause) = decision.pause {
        return stalled::on_pause(
            runtime,
            store,
            run_id,
            owner,
            task_universe,
            llm,
            &run_dir,
            &mut record,
            &mut frozen,
            &path,
            &decision,
            cause,
        )
        .await;
    }
    let mut result = result_for(&record, &relative_record_path(&run_dir, &path), &decision);
    result::with_task_scope(&mut result, task_universe, &record);
    Ok(with_frozen_identity(&record, frozen.as_ref(), result))
}

include!("workflow_live_v3_acceptance_test_modules.rs");

#[path = "workflow_live_v3_acceptance_evaluate.rs"]
mod evaluation;
use evaluation::evaluate;
