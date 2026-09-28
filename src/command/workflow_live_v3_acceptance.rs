//! The authored run's acceptance stage: host side of `acceptance-contract-run`.
//!
//! Obs-32. The v3 script's final primitive, `await acceptance()`, asks the host
//! to run every check in the task set's frozen `acceptance-contract.json`
//! against the repository as the run left it. This is that host call: it
//! resolves where the checks run (`workflow_live_v3_acceptance_exec`), runs
//! the WHOLE contract every round (the script's `checkIds` are validated but
//! never narrow the run), maps each failing check to the tasks whose
//! `implements` list names it, writes an append-only round record under
//! `v2/acceptance/<round>/`, and answers the script with the failing checks
//! and whether the round is final. The finalizer reads the record the last
//! round's own call record names; the script cannot mark anything passed.
//!
//! A persisted script call, but never a replayed one: the host re-executes
//! every acceptance round, on resume too, because a stored round describes a
//! repository that may since have changed. Run-control polling is as for
//! every other call, so a pause during the stage re-enters it on resume.

use std::path::Path;

use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::AcceptanceCriterion;
use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::acceptance_stage::{
    ACCEPTANCE_MAX_ROUNDS, ACCEPTANCE_ROUND_RECORD_SCHEMA_VERSION, ACCEPTANCE_STAGE_TOOL,
    AcceptanceCheckRecordV1, AcceptanceCheckStatus, AcceptanceRoundRecordV1, next_attempt,
    owning_tasks, relative_record_path, round_dir, write_round_record,
};
use archon_workflow::{
    WorkflowError, WorkflowResult, WorkflowStore, WorkflowV2CallExecution, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2Status,
    poll_v2_run_control,
};

use super::WorkflowV2ScriptRuntime;
#[path = "workflow_live_v3_acceptance_exec.rs"]
mod exec;
#[path = "workflow_live_v3_acceptance_output.rs"]
mod output;
#[path = "workflow_live_v3_acceptance_regression.rs"]
mod regression;
#[path = "workflow_live_v3_acceptance_repair.rs"]
mod repair;
use output::{tail, write_output_files};

pub(super) fn is_acceptance_stage_call(execution: &WorkflowV2CallExecution) -> bool {
    archon_workflow::v2::script::is_acceptance_stage_call(&execution.call)
}

struct StageRequest {
    round: u32,
    max_rounds: u32,
    check_ids: Vec<String>,
}

fn parse_request(execution: &WorkflowV2CallExecution) -> WorkflowResult<StageRequest> {
    let extra = &execution.call.options.extra;
    let round = extra
        .get("round")
        .and_then(serde_json::Value::as_u64)
        .filter(|round| *round >= 1)
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "{ACCEPTANCE_STAGE_TOOL} requires a positive `round`; the prelude's acceptance() primitive supplies it"
            ))
        })?;
    let max_rounds = extra
        .get("maxRounds")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(u64::from(ACCEPTANCE_MAX_ROUNDS))
        .clamp(1, u64::from(ACCEPTANCE_MAX_ROUNDS));
    let check_ids = extra
        .get("checkIds")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect();
    Ok(StageRequest {
        round: u32::try_from(round).unwrap_or(u32::MAX),
        max_rounds: max_rounds as u32,
        check_ids,
    })
}

/// Run one acceptance round for the authored run and record it.
pub(super) async fn run_acceptance_stage(
    runtime: &WorkflowV2ScriptRuntime,
    execution: &WorkflowV2CallExecution,
    store: &WorkflowStore,
    run_id: &str,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    llm: Option<&dyn archon_workflow::WorkflowLlmClient>,
) -> WorkflowResult<WorkflowV2Result> {
    let request = parse_request(execution)?;
    let call_id = execution.call.id.clone();
    let run_dir = store.run_dir(run_id);
    let attempt = next_attempt(&run_dir, request.round);
    let mut record = AcceptanceRoundRecordV1 {
        schema_version: ACCEPTANCE_ROUND_RECORD_SCHEMA_VERSION,
        run_id: run_id.to_string(),
        call_id: call_id.clone(),
        round: request.round,
        attempt,
        max_rounds: request.max_rounds,
        contract_present: false,
        requested_check_ids: request.check_ids.clone(),
        execution: None,
        checks: Vec::new(),
        operational_errors: Vec::new(),
        contract_repairs: Vec::new(),
        final_round: true,
    };
    let evaluation = archon_workflow::control_race::until_run_stops(
        store,
        run_id,
        &call_id,
        evaluate(
            runtime,
            store,
            run_id,
            &call_id,
            task_universe,
            llm,
            &request,
            &run_dir,
            &mut record,
        ),
    )
    .await;
    // A pause or cancel unwinds without a record: the round re-enters on
    // resume as the next attempt. Anything else is the round's own outcome.
    evaluation?;
    record.final_round = record.failing_checks().is_empty()
        || request.round >= request.max_rounds
        || !record.has_remediable_failures()
        || !record.operational_errors.is_empty();
    let path = write_round_record(&run_dir, &record)?;
    Ok(result_for(&record, &relative_record_path(&run_dir, &path)))
}

#[allow(clippy::too_many_arguments)]
async fn evaluate(
    runtime: &WorkflowV2ScriptRuntime,
    store: &WorkflowStore,
    run_id: &str,
    call_id: &str,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    llm: Option<&dyn archon_workflow::WorkflowLlmClient>,
    request: &StageRequest,
    run_dir: &Path,
    record: &mut AcceptanceRoundRecordV1,
) -> WorkflowResult<()> {
    let context = match exec::resolve_context(
        store,
        run_id,
        runtime.target_repository_root.as_deref(),
        task_universe,
    ) {
        Ok(context) => context,
        Err(error) => {
            record.operational_errors.push(error.to_string());
            return Ok(());
        }
    };
    record.execution = Some(context.execution_record());
    if !context.contract_path().exists() {
        record.contract_present = false;
        // Absent is a vacuous pass ONLY for a task set that never declared a
        // contract. One that froze or pinned one, or whose tasks name the
        // checks they implement, has lost it — and a lost contract is not a
        // passed one.
        if let Some(declared) = exec::contract_declaration(&context, task_universe) {
            record.operational_errors.push(format!(
                "{} is missing although the task set declares an acceptance contract ({declared})",
                context.contract_path().display()
            ));
        }
        return Ok(());
    }
    record.contract_present = true;
    let (mut contract, mut chain_digest, frozen) = match exec::load_contract(&context) {
        Ok(loaded) => loaded,
        Err(error) => {
            record
                .operational_errors
                .push(format!("frozen acceptance contract is not usable: {error}"));
            return Ok(());
        }
    };
    if !frozen {
        record.operational_errors.push(format!(
            "{} has no acceptance-contract.lock: the contract is not frozen, so its checks cannot be trusted as the task set's acceptance; run the freeze before implementation",
            context.contract_path().display()
        ));
        return Ok(());
    }
    for requested in &request.check_ids {
        if !contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .any(|criterion| &criterion.id == requested)
        {
            record.operational_errors.push(format!(
                "requested acceptance check '{requested}' is not in the frozen contract"
            ));
        }
    }
    if !record.operational_errors.is_empty() {
        return Ok(());
    }
    // A check the judge did not accept can never run: repair the contract
    // before running it, and never hand it to the implementing tasks.
    let Some(mut defects) =
        repair::apply(llm, &context, &mut contract, &mut chain_digest, record).await
    else {
        return Ok(());
    };
    let evidence_dir =
        round_dir(run_dir, request.round).join(format!("attempt-{:02}", record.attempt));
    // Every round runs the WHOLE contract. The script names the checks it
    // expects a remediation to have fixed, but a fix can regress a check that
    // passed in an earlier round, and a round that re-ran only the named ones
    // would record that regression nowhere: the final round must cover every
    // check for its verdict to be the contract's. A contract defect is the one
    // exception: it cannot run, so it is recorded instead of executed.
    let mut results = {
        let selected: Vec<&AcceptanceCriterion> = (contract.acceptance.iter())
            .chain(&contract.supplementary)
            .filter(|criterion| !defects.contains_key(&criterion.id))
            .collect();
        poll_v2_run_control(store, run_id, call_id)?;
        let ran = exec::execute_checks(
            store,
            run_id,
            call_id,
            &context,
            &contract,
            &chain_digest,
            &selected,
            &evidence_dir,
        )
        .await?;
        (selected.iter().map(|criterion| criterion.id.clone()))
            .zip(ran)
            .collect::<std::collections::BTreeMap<String, CheckResult>>()
    };
    // A check that crashed in its own code is a contract defect too: it is
    // repaired, republished and re-run in this round, never handed to a task.
    let round = repair::Round {
        llm,
        context: &context,
        store,
        run_id,
        call_id,
        evidence_dir: &evidence_dir,
    };
    (repair::repair_crashed(
        &round,
        &mut contract,
        &mut chain_digest,
        &mut results,
        record,
    ))
    .await?
    .into_iter()
    .for_each(|(id, defect)| drop(defects.insert(id, defect)));
    let all: Vec<&AcceptanceCriterion> = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .collect();
    let selected: Vec<&AcceptanceCriterion> = (all.iter().copied())
        .filter(|criterion| !defects.contains_key(&criterion.id))
        .collect();
    for criterion in &all {
        let result = results.remove(&criterion.id);
        if let Some(result) = &result {
            write_output_files(&evidence_dir, result);
        }
        if let Some(defect) = defects.get(&criterion.id) {
            record
                .checks
                .push(repair::defect_record(criterion, defect, result.as_ref()));
            continue;
        }
        let Some(result) = result else {
            continue;
        };
        record
            .checks
            .push(check_record(criterion, &result, task_universe));
    }
    // A failing check that held at its owner's landing names the landing
    // that broke it, so the remediation reaches the task that can fix it.
    poll_v2_run_control(store, run_id, call_id)?;
    regression::attribute(
        &context,
        &selected,
        &chain_digest,
        run_dir,
        &evidence_dir,
        record,
    )
    .await;
    // And each failing check names the files its failure implicates and who
    // can write them, so no routed unit is left unable to write the fix.
    let texts = all
        .iter()
        .map(|c| {
            (
                c.id.clone(),
                archon_workflow::v2::acceptance_routing::check_command(c),
            )
        })
        .collect();

    archon_workflow::v2::acceptance_routing::route_failures(
        task_universe,
        &context.repository,
        &texts,
        record,
    );
    Ok(())
}

fn check_record(
    criterion: &AcceptanceCriterion,
    result: &CheckResult,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> AcceptanceCheckRecordV1 {
    let status = if result.operational_error.is_some() {
        AcceptanceCheckStatus::Error
    } else if result.exit_code == Some(0) {
        AcceptanceCheckStatus::Passed
    } else {
        AcceptanceCheckStatus::Failed
    };
    AcceptanceCheckRecordV1 {
        check_id: criterion.id.clone(),
        criterion: criterion.criterion.clone(),
        kind: exec::check_kind(criterion).to_string(),
        status,
        exit_code: result.exit_code,
        operational_error: result.operational_error.clone(),
        owning_tasks: owning_tasks(universe, &criterion.id),
        stdout_tail: tail(&result.stdout),
        stderr_tail: tail(&result.stderr),
        regressed_by: None,
        contract_defect: false,
        routing: None,
    }
}

/// The call result. Status is `Accepted` while the stage can still act (a
/// clean round, or a failing round remediation will follow) and
/// `NeedsReview` for a final round that still fails — so the record is
/// re-executed rather than replayed on resume, and the run's own status
/// merge agrees with the finalizer's gate.
fn result_for(record: &AcceptanceRoundRecordV1, record_path: &str) -> WorkflowV2Result {
    let failing = record.failing_checks();
    let blocks = record.blocks_completion();
    let status = if blocks && record.final_round {
        WorkflowV2Status::NeedsReview
    } else {
        WorkflowV2Status::Accepted
    };
    let failing_view: Vec<serde_json::Value> = failing
        .iter()
        .map(|check| {
            serde_json::json!({
                "check_id": check.check_id,
                "criterion": check.criterion,
                "kind": check.kind,
                "status": check.status,
                "exit_code": check.exit_code,
                "operational_error": check.operational_error,
                "owning_tasks": check.owning_tasks,
                "regressed_by": check.regressed_by,
                "routing": check.routing,
                "contract_defect": check.contract_defect,
                "stdout_tail": check.stdout_tail,
                "stderr_tail": check.stderr_tail,
            })
        })
        .collect();
    let summary = if !record.contract_present && record.operational_errors.is_empty() {
        format!(
            "acceptance round {}: no acceptance-contract.json at the task set root; nothing to check",
            record.round
        )
    } else if !record.operational_errors.is_empty() {
        format!(
            "acceptance round {} could not evaluate: {}",
            record.round,
            record.operational_errors.join("; ")
        )
    } else {
        format!(
            "acceptance round {}: {} passed, {} failed{} ({} checks run, {})",
            record.round,
            record.passed_check_ids().len(),
            failing.len(),
            if failing.is_empty() {
                String::new()
            } else {
                format!(" [{}]", record.failing_check_ids().join(", "))
            },
            record.checks.len(),
            record
                .execution
                .as_ref()
                .map_or("no execution site", |execution| execution.mode.as_str())
        )
    };
    let mut result = WorkflowV2Result {
        status,
        summary: summary.clone(),
        data: serde_json::json!({
            "round": record.round,
            "attempt": record.attempt,
            "max_rounds": record.max_rounds,
            "final": record.final_round,
            "contract_present": record.contract_present,
            "record_path": record_path,
            "execution_mode": record.execution.as_ref().map(|execution| execution.mode.clone()),
            "failing": failing_view,
            "passed": record.passed_check_ids(),
            "unowned_failing_check_ids": record.unowned_failing_check_ids(),
            "contract_defect_check_ids": record.contract_defect_ids(),
            "contract_repairs": record.contract_repairs,
            "operational_errors": record.operational_errors,
        }),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        summary,
    ));
    for check in &record.checks {
        result
            .commands_run
            .push(archon_workflow::WorkflowV2CommandRecord {
                kind: archon_workflow::WorkflowV2CommandKind::Test,
                command: format!("acceptance-check:{}", check.check_id),
                status: if check.failing() {
                    archon_workflow::WorkflowV2CommandStatus::Failed
                } else {
                    archon_workflow::WorkflowV2CommandStatus::Succeeded
                },
                exit_code: check.exit_code,
                output_summary: check
                    .operational_error
                    .clone()
                    .unwrap_or_else(|| check.stderr_tail.chars().take(400).collect()),
                pre_existing: false,
            });
    }
    for check in &failing {
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: format!("acceptance-{}", check.check_id),
            description: format!(
                "frozen acceptance check {} {} ({}): {}{}",
                check.check_id,
                if check.contract_defect {
                    "is a contract defect"
                } else {
                    "failed"
                },
                check.operational_error.as_deref().unwrap_or("nonzero exit"),
                check.criterion,
                check
                    .regressed_by
                    .as_ref()
                    .map_or(String::new(), |regression| format!(
                        "; it held at {} and regressed at landing {} ({}) of {}",
                        regression.held_at,
                        regression.landing_commit,
                        regression.landing_stage,
                        regression.tasks.join(", ")
                    )),
            ) + check
                .routing
                .as_ref()
                .map_or(String::new(), |r| r.describe())
                .as_str(),
            severity: Some("high".to_string()),
        });
    }
    for error in &record.operational_errors {
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: "acceptance-stage-operational-error".to_string(),
            description: error.clone(),
            severity: Some("high".to_string()),
        });
    }
    result
}

// The stage runs its checks through the POSIX process-group runner; the
// tests execute `test -f` criteria for real, so they are Unix-only like it.
#[cfg(all(test, unix))]
#[path = "workflow_live_v3_acceptance_crash_tests.rs"]
mod crash_tests;
#[cfg(all(test, unix))]
#[path = "workflow_live_v3_acceptance_repair_tests.rs"]
mod repair_tests;
#[cfg(all(test, unix))]
#[path = "workflow_live_v3_acceptance_tests.rs"]
mod tests;
