//! The acceptance round's answer to the script, split from
//! `workflow_live_v3_acceptance` for size. Batch J: each failing check's
//! view carries what the regression search found (`regression_search`) and
//! whether no unit can fix it (`blocked`); a blocked check is raised as a
//! HIGH operational finding of its own, naming the rule that blocks it.
//!
//! Batch O: `final` is the host's progress rule (`acceptance_progress`),
//! never a round count; `escalate` says the last round made no progress, so
//! its failing checks go to all their owners as one cross-owner unit.
//! A failing entry with `owning_tasks` is a task's to fix (the host fills
//! them, reassigning a check no `implements` names); an entry without them
//! (status `error`, or a contract defect) is the host's, repaired before
//! the next round runs.

use archon_workflow::v2::acceptance_stage::AcceptanceRoundRecordV1;
use archon_workflow::v2::acceptance_stage::progress::LoopDecision;
use archon_workflow::{
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2ResidualGap, WorkflowV2Result,
    WorkflowV2Status,
};

use super::output::brief;

/// Residual gap id prefix of a failed check no unit can fix.
pub(super) const BLOCKED_GAP_PREFIX: &str = "acceptance-blocked-";

/// The call result. Status is `Accepted` while the stage can still act (a
/// clean round, or a failing round remediation will follow) and
/// `NeedsReview` for a final round that still fails — so the record is
/// re-executed rather than replayed on resume, and the run's own status
/// merge agrees with the finalizer's gate.
pub(super) fn result_for(
    record: &AcceptanceRoundRecordV1,
    record_path: &str,
    decision: &LoopDecision,
) -> WorkflowV2Result {
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
                // Remediated by a task unit: it ran, failed, and names owners.
                "remediable": check.ran_and_failed() && !check.owning_tasks.is_empty(),
                "regressed_by": check.regressed_by,
                "regression_search": check.regression_search,
                "routing": check.routing,
                "blocked": check.blocked,
                "contract_defect": check.contract_defect,
                "stdout_tail": check.stdout_tail,
                "stderr_tail": check.stderr_tail,
            })
        })
        .collect();
    let summary = if !record.contract_present && record.operational_errors.is_empty() {
        format!(
            "acceptance round {}: no acceptance-contract.json at the task set root; no check ran, which never completes the run",
            record.round
        )
    } else if !record.operational_errors.is_empty() && record.checks.is_empty() {
        format!(
            "acceptance round {} could not evaluate: {}",
            record.round,
            record.operational_errors.join("; ")
        )
    } else {
        format!(
            "acceptance round {}: {} passed, {} failed{} ({} checks run, {}){}",
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
                .map_or("no execution site", |execution| execution.mode.as_str()),
            if record.operational_errors.is_empty() {
                String::new()
            } else {
                format!("; {}", record.operational_errors.join("; "))
            }
        )
    };
    let blocked: Vec<String> = (record.blocked_checks().into_iter())
        .map(|(id, _)| id)
        .collect();
    let mut result = WorkflowV2Result {
        status,
        summary: summary.clone(),
        data: serde_json::json!({
            "round": record.round,
            "attempt": record.attempt,
            "max_rounds": record.max_rounds,
            "final": record.final_round,
            "escalate": decision.escalate,
            "stalled_rounds": decision.stalled_rounds,
            "task_remediable_check_ids": record.task_remediable_check_ids(),
            "contract_present": record.contract_present,
            "record_path": record_path,
            "execution_mode": record.execution.as_ref().map(|execution| execution.mode.clone()),
            "failing": failing_view,
            "passed": record.passed_check_ids(),
            "unowned_failing_check_ids": record.unowned_failing_check_ids(),
            "contract_defect_check_ids": record.contract_defect_ids(),
            "blocked_check_ids": blocked,
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
                    .unwrap_or_else(|| brief(&check.stderr_tail)),
                pre_existing: false,
            });
    }
    for check in &failing {
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: format!("acceptance-{}", check.check_id),
            description: format!(
                "frozen acceptance check {} {} ({}): {}{}{}",
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
                check
                    .regression_search
                    .as_ref()
                    .map_or(String::new(), |search| format!(
                        "; regression search: {}",
                        search.note
                    )),
            ) + archon_workflow::v2::acceptance_routing::clause(check).as_str(),
            severity: Some("high".to_string()),
        });
    }
    // Batch J (d): a failed check no unit can fix is an operational finding
    // of its own, naming the rule, so the operator sees why no round was
    // sent for it.
    for (id, rule) in record.blocked_checks() {
        tracing::warn!(check = %id, "acceptance check blocked: {rule}");
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: format!("{BLOCKED_GAP_PREFIX}{id}"),
            description: format!(
                "operational: frozen acceptance check {id} failed and no remediation unit can fix it, so none is sent: {rule}"
            ),
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

/// Issue 262: pauses the run because the acceptance loop stalled after
/// `record` (or hit its runaway guard), and returns the control error the
/// round ends with. The round's record is the evidence; a resume runs the
/// round again, as the next attempt, on whatever changed meanwhile. Only
/// `generation`, the one the round started under, may pause the run.
pub(super) fn pause_on_stall(
    store: &archon_workflow::WorkflowStore,
    run_id: &str,
    generation: u64,
    record: &AcceptanceRoundRecordV1,
    record_path: &str,
    decision: &LoopDecision,
    cause: &'static str,
) -> archon_workflow::WorkflowError {
    let failing = record.failing_check_ids();
    let resume = format!("archon workflow resume --live --yes {run_id}");
    let message = format!(
        "acceptance round {} (attempt {}) made no progress ({cause}: {} trailing round(s) without progress, {} check(s) still failing: {}); the run is paused, not failed. The evidence is {record_path}; fix what it names, then {resume}",
        record.round,
        record.attempt,
        decision.stalled_rounds,
        failing.len(),
        failing.join(", ")
    );
    let detail = serde_json::json!({
        "event": "acceptance_stall_pause",
        "round": record.round,
        "attempt": record.attempt,
        "cause": cause,
        "stalled_rounds": decision.stalled_rounds,
        "failing_check_ids": failing,
        "operational_errors": record.operational_errors,
        "record_path": record_path,
        "resume": resume,
    });
    // Owned by the generation the round started under: a round an operator
    // pause and resume made obsolete stops instead of pausing the new owner.
    match archon_workflow::control_pause::pause_with_evidence(store, run_id, generation, detail) {
        Ok(event) => {
            if let Err(error) = event {
                tracing::warn!(%error, "acceptance stall pause event not recorded");
            }
            tracing::warn!(run_id, "{message}");
            archon_workflow::WorkflowError::ControlPaused(message)
        }
        Err(error) => error,
    }
}

/// A declared entry as a path: its first path-like token, without code
/// quotes or a leading `./`; never a glob.
fn declared_file(entry: &str) -> Option<String> {
    let token = entry
        .split_whitespace()
        .find(|token| token.contains('/') || token.contains('.'))?
        .trim_matches(|c| c == '`' || c == '"' || c == '\'' || c == ',');
    let token = token.strip_prefix("./").unwrap_or(token);
    (!token.is_empty() && !token.contains('*') && !token.contains("://")).then(|| token.to_string())
}

/// REM-13: each task a failing check's unit names, with its own file and
/// declared files (`task_files`, `task_scope`), so a script that names none
/// (one the prelude runs acceptance for) can still form its units. A script
/// that names its own is never read these.
pub(super) fn with_task_scope(
    result: &mut WorkflowV2Result,
    universe: Option<&archon_workflow::task_universe::WorkflowV2TaskUniverse>,
    record: &AcceptanceRoundRecordV1,
) {
    let Some(universe) = universe else {
        return;
    };
    let named: std::collections::BTreeSet<&str> = (record.failing_checks().into_iter())
        .flat_map(|check| {
            (check.owning_tasks.iter())
                .chain(check.regressed_by.iter().flat_map(|r| r.tasks.iter()))
                .chain(check.routing.iter().flat_map(|r| r.writer_tasks.iter()))
                .map(String::as_str)
                .collect::<Vec<_>>()
        })
        .collect();
    let mut files = serde_json::Map::new();
    let mut scope = serde_json::Map::new();
    for task in (universe.tasks.iter()).filter(|t| named.contains(t.canonical_task_id.as_str())) {
        files.insert(
            task.canonical_task_id.clone(),
            serde_json::json!(task.source_path),
        );
        let declared: Vec<String> = (task.files_expected_to_change.iter())
            .chain(&task.shared_append_target_files)
            .filter_map(|entry| declared_file(entry))
            .collect();
        scope.insert(task.canonical_task_id.clone(), serde_json::json!(declared));
    }
    if files.is_empty() {
        return;
    }
    result.data["task_files"] = serde_json::Value::Object(files);
    result.data["task_scope"] = serde_json::Value::Object(scope);
}
