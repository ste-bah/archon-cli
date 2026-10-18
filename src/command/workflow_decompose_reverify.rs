//! Read-only re-judging of a completed fixed decomposition.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_core::{config::ArchonConfig, env_vars::ArchonEnvVars};
use archon_workflow::{
    DecompositionPhase, FixedDecompositionStateV1, HostCommandRequest, RunStatus,
    WorkflowEventKind, WorkflowEventLog, WorkflowStore, WorkflowV2CallRecord, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result, WorkflowV2ResultStore,
    WorkflowV2Status, task_set_contract::content_digest,
};

use super::workflow_decompose_reverify_assessment::executor_error_result;
pub(super) use super::workflow_decompose_reverify_assessment::gate_assessment;
use super::workflow_decompose_reverify_gates::execute_gate;
use crate::command::workflow_host_command_catalog::HostCommandResolutionContext;
use crate::command::workflow_host_command_exec::FixedHostCommandExecutor;

#[path = "workflow_decompose_reverify_eligibility.rs"]
pub(crate) mod eligibility;

#[path = "workflow_decompose_reverify_persist_call.rs"]
pub(crate) mod persist;
pub(super) use persist::persist_call;

#[path = "workflow_decompose_reverify_publish.rs"]
pub(crate) mod publish;

#[path = "workflow_decompose_reverify_acceptance.rs"]
mod acceptance;
#[path = "workflow_decompose_reverify_types.rs"]
mod types;
pub(super) use types::GateResult;
use types::{ReverifyReport, SubjectResult};

pub(crate) async fn reverify(
    cwd: &Path,
    run_id: &str,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<String> {
    if run_id.trim().is_empty() || run_id.contains('/') || run_id.contains('\\') {
        return Err(anyhow!("invalid workflow run id '{run_id}'"));
    }
    let project_root = cwd.canonicalize().context("resolving project root")?;
    let store = WorkflowStore::project(&project_root);
    let run_dir = store.run_dir(run_id);
    let _lease = crate::command::workflow_executor_lease::acquire(&run_dir, run_id)?;
    let run = store.load_state(run_id)?;
    let invocation_generation = run.generation;
    let invocation_status = run.status.clone();
    let state_path = run_dir.join(types::STATE_PATH);
    let state: FixedDecompositionStateV1 = serde_json::from_slice(
        &std::fs::read(&state_path).with_context(|| format!("reading {}", state_path.display()))?,
    )?;
    if state.run_kind != archon_workflow::WorkflowRunKind::FixedDecompositionV1
        || state.phase != DecompositionPhase::Completed
    {
        return Err(anyhow!(
            "workflow {run_id} is not a completed fixed decomposition"
        ));
    }
    let v2 = WorkflowV2ResultStore::new(run_dir.join("v2"));
    let latest = crate::command::workflow_decompose_latest_report::latest_report(&store, run_id)?;
    let reverify_paused = eligibility::check_run_status(
        &store,
        &run_dir,
        run_id,
        &_lease,
        run.status,
        latest.as_ref(),
        &v2,
    )?;
    if let Some(response) = eligibility::recover_accepted_report_status(
        &store,
        run_id,
        reverify_paused,
        invocation_generation,
        &invocation_status,
        latest.as_ref(),
    )? {
        return Ok(response);
    }
    let latest =
        latest.ok_or_else(|| anyhow!("workflow {run_id} has no valid prior final report"))?;
    let prior = v2
        .load_call_records()?
        .into_iter()
        .find(|record| {
            record.call.id == latest.id && record.call.method == WorkflowV2HostMethod::FinalReport
        })
        .ok_or_else(|| {
            anyhow!(
                "workflow {run_id} latest report {} has no call record",
                latest.id
            )
        })?;
    let prior_digest = prior.output_hash.clone();
    let project_root = PathBuf::from(&state.identity.project_root_identity);
    let prd_path = PathBuf::from(&state.identity.prd_identity);
    let task_root = PathBuf::from(&state.identity.task_root_identity);
    let pin_path =
        crate::command::workflow_task_set::acceptance_pin_path(&project_root, &task_root);
    let _chain_lock = if task_root.is_dir() {
        let lock = archon_workflow::task_set_publish_lock::PublishLockFile::hold_shared_existing(
            &pin_path,
        )
        .map_err(|error| anyhow!(error))?;
        let [journal, _] = archon_workflow::task_set_publish_lock::journal_paths(&pin_path);
        if journal.exists() {
            return Err(anyhow!(
                "reverify cannot inspect frozen task chain while publish recovery is pending at {}; run a live resume to settle it",
                journal.display()
            ));
        }
        lock
    } else {
        None
    };
    let prd_bytes = std::fs::read(&prd_path)?;
    let prd_digest = content_digest(&prd_bytes);
    let skeleton_path = task_root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE);
    let skeleton: archon_workflow::task_skeleton::TaskSkeleton = serde_json::from_slice(
        &std::fs::read(&skeleton_path)
            .with_context(|| format!("reading {}", skeleton_path.display()))?,
    )?;
    let catalog = crate::command::workflow_host_command_catalog::fixed_decomposition_catalog(
        &state.identity.starting_binary_revision,
    )?;
    let program = std::env::current_exe()?.canonicalize()?;
    let base_context = HostCommandResolutionContext {
        program,
        project_root: project_root.clone(),
        prd_path: prd_path.clone(),
        prd_digest,
        task_root: task_root.clone(),
        run_staging_root: run_dir.join("host-command-staging"),
        frozen_task_id: None,
        frozen_task_file: None,
        freeze_provider_environment:
            crate::command::workflow_decompose::freeze_provider_environment(env_vars),
        acceptance_environment_allowlist: config
            .workflow
            .acceptance_execution
            .as_ref()
            .map(|policy| policy.environment_allowlist.clone())
            .unwrap_or_default(),
        gate_mode: config.workflow.gate_mode,
    };
    let event_seq = store.next_event_seq(run_id)?;
    let invocation_id = format!("reverify-{event_seq}");
    WorkflowEventLog::new(store.clone()).emit(
        run_id,
        event_seq,
        WorkflowEventKind::DecompositionReverifyStarted,
        serde_json::json!({"invocation_id": invocation_id,
            "lease_pid": _lease.holder().pid, "lease_acquired_at": _lease.holder().acquired_at}),
    )?;
    let paused_authorization = reverify_paused.then(|| {
        crate::command::workflow_host_command_operational::PausedRunAuthorization::new(
            invocation_id.clone(),
            run.generation,
            &_lease,
        )
    });
    let executor =
        FixedHostCommandExecutor::new(catalog.clone(), base_context.clone(), run_dir.clone());
    let executor = if let Some(authorization) = paused_authorization.clone() {
        executor.authorize_paused_run(authorization)
    } else {
        executor
    };
    let mut call_ids = Vec::new();
    let mut subjects = Vec::new();
    let mut set_gates = Vec::new();
    let mut frozen_gates = Vec::new();
    let mut failed_subjects = Vec::new();
    let mut all_accepted = true;
    let mut operational_failure = false;
    let acceptance =
        match acceptance::reverify_checks(&project_root, &prd_path, &task_root, config, env_vars)
            .await
        {
            Ok(acceptance) => acceptance,
            Err(error) => {
                return Err(publish::record_operational_failure(
                    &store,
                    run_id,
                    &run_dir,
                    &_lease,
                    invocation_generation,
                    &invocation_status,
                    &v2,
                    &prior.call.id,
                    &prior_digest,
                    &invocation_id,
                    &error,
                ));
            }
        };
    let checks = acceptance.checks;
    operational_failure |= acceptance.operational_failure;
    failed_subjects.extend(acceptance.task_subjects);
    all_accepted &= acceptance.accepted;
    for (index, frozen) in skeleton.tasks.iter().enumerate() {
        let path = task_root.join(&frozen.file_name);
        let bytes = std::fs::read(&path)
            .with_context(|| format!("reading task body {}", path.display()))?;
        let disk_digest = content_digest(&bytes);
        let body = match String::from_utf8(bytes.clone()) {
            Ok(body) => body,
            Err(error) => {
                let cause = format!("task body is not valid UTF-8: {error}");
                let request = HostCommandRequest::new("land-task-body", None)?;
                let result = super::workflow_decompose_reverify_assessment::input_invalid_result(
                    &frozen.task_id,
                    &cause,
                );
                let call_id = format!("reverify-{event_seq}-body-{index}");
                let stamped = persist_call(
                    &executor,
                    &store,
                    &v2,
                    run_id,
                    &run_dir,
                    &_lease,
                    invocation_generation,
                    &invocation_status,
                    &call_id,
                    request,
                    &result,
                    &bytes,
                    &[],
                )?;
                call_ids.push(call_id);
                failed_subjects.push(frozen.task_id.clone());
                all_accepted = false;
                subjects.push(SubjectResult {
                    task_id: frozen.task_id.clone(),
                    file_name: frozen.file_name.clone(),
                    disk_digest: disk_digest.clone(),
                    judged_digest: disk_digest,
                    logic_version: stamped["logicVersion"].as_u64().map(|value| value as u32),
                    logic_digest: stamped["logicDigest"].as_str().map(str::to_string),
                    build: stamped["logicBuild"].as_str().map(str::to_string),
                    verdict: "needs_remediation".into(),
                    findings: vec![cause],
                });
                continue;
            }
        };
        let request = HostCommandRequest::new("land-task-body", Some(body.clone()))?;
        let mut context = base_context.clone();
        context.frozen_task_id = Some(frozen.task_id.clone());
        context.frozen_task_file = Some(path);
        let body_executor =
            FixedHostCommandExecutor::new(catalog.clone(), context, run_dir.clone());
        let body_executor = if let Some(authorization) = paused_authorization.clone() {
            body_executor.authorize_paused_run(authorization)
        } else {
            body_executor
        };
        eprintln!(
            "reverify gate started subject={} capability=land-task-body",
            frozen.task_id
        );
        let result = body_executor
            .execute_judge_only(request.clone(), Some(run.generation))
            .await
            .unwrap_or_else(|error| {
                executor_error_result(
                    error,
                    crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
                    &frozen.task_id,
                )
            });
        let (verdict, findings) = gate_assessment(&result);
        eprintln!(
            "reverify gate finished subject={} capability=land-task-body verdict={verdict}",
            frozen.task_id
        );
        let call_id = format!("reverify-{event_seq}-body-{index}");
        let stamped = persist_call(
            &body_executor,
            &store,
            &v2,
            run_id,
            &run_dir,
            &_lease,
            invocation_generation,
            &invocation_status,
            &call_id,
            request,
            &result,
            body.as_bytes(),
            &[],
        )?;
        call_ids.push(call_id.clone());
        operational_failure |= verdict == "operational_failure";
        let accepted = result.reusable();
        all_accepted &= accepted;
        if !accepted && verdict == "needs_remediation" {
            failed_subjects.push(frozen.task_id.clone());
        }
        subjects.push(SubjectResult {
            task_id: frozen.task_id.clone(),
            file_name: frozen.file_name.clone(),
            disk_digest: disk_digest.clone(),
            judged_digest: content_digest(body.as_bytes()),
            logic_version: stamped["logicVersion"].as_u64().map(|value| value as u32),
            logic_digest: stamped["logicDigest"].as_str().map(str::to_string),
            build: stamped["logicBuild"].as_str().map(str::to_string),
            verdict,
            findings,
        });
    }
    for (index, command_id) in ["task-set-lint", "requirements-trace"]
        .into_iter()
        .enumerate()
    {
        let (result, gate) = execute_gate(
            &executor,
            &store,
            &v2,
            run_id,
            &run_dir,
            &_lease,
            run.generation,
            &invocation_status,
            event_seq,
            index,
            command_id,
            &[],
        )
        .await?;
        all_accepted &= result.reusable();
        operational_failure |= gate.verdict == "operational_failure";
        if gate.verdict == "needs_remediation" {
            failed_subjects.extend(gate.subjects.iter().cloned());
        }
        call_ids.push(gate.call_id.clone());
        set_gates.push(gate);
    }
    for (index, command_id) in ["verify-frozen-acceptance", "verify-frozen-skeleton"]
        .into_iter()
        .enumerate()
    {
        let (result, gate) = execute_gate(
            &executor,
            &store,
            &v2,
            run_id,
            &run_dir,
            &_lease,
            run.generation,
            &invocation_status,
            event_seq,
            index,
            command_id,
            &[],
        )
        .await?;
        all_accepted &= result.reusable();
        operational_failure |= gate.verdict == "operational_failure";
        if gate.verdict == "needs_remediation" {
            failed_subjects.extend(gate.subjects.iter().cloned());
        }
        call_ids.push(gate.call_id.clone());
        frozen_gates.push(gate);
    }
    let outcome = if all_accepted {
        "accepted"
    } else if operational_failure {
        "operational_failure"
    } else {
        "needs_remediation"
    };
    let report_event_seq = store.next_event_seq(run_id)?;
    let report_id = format!("reverify-{event_seq}-report");
    let mut call_record_ids_at_report = v2
        .load_call_records()?
        .into_iter()
        .map(|record| record.call.id)
        .collect::<Vec<_>>();
    call_record_ids_at_report.push(report_id.clone());
    call_record_ids_at_report.sort();
    let report = ReverifyReport {
        schema_version: 1,
        id: report_id.clone(),
        supersedes_id: prior.call.id.clone(),
        supersedes_digest: prior_digest,
        outcome: outcome.to_string(),
        report_event_seq,
        call_record_ids_at_report,
        calls: call_ids.clone(),
        subjects,
        set_gates,
        frozen_gates,
        checks,
        operational_failure_cause: None,
    };
    let report_bytes = serde_json::to_vec_pretty(&report)?;
    if v2.result_path(&report_id).exists()
        || run_dir
            .join(format!("{}/{report_id}.json", types::REPORT_PATH))
            .exists()
    {
        return Err(anyhow!("reverify report already exists: {report_id}"));
    }
    let report_digest = content_digest(&report_bytes);
    let report_record = WorkflowV2CallRecord::new(
        run_id,
        WorkflowV2HostCall {
            id: report_id.clone(),
            method: WorkflowV2HostMethod::FinalReport,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        1,
        report_digest.clone(),
        WorkflowV2Result {
            status: if all_accepted {
                WorkflowV2Status::Accepted
            } else {
                WorkflowV2Status::NeedsReview
            },
            summary: format!("reverify outcome: {outcome}"),
            data: serde_json::to_value(&report)?,
            ..WorkflowV2Result::default()
        },
        Vec::new(),
    );
    if v2.result_path(&report_id).exists() {
        return Err(anyhow!(
            "reverify report call record already exists: {report_id}"
        ));
    }
    let failed_subjects = if !operational_failure && !failed_subjects.is_empty() {
        Some(crate::command::workflow_decompose_remediation::deduplicate_subjects(failed_subjects))
    } else {
        None
    };
    let operational_cause = publish::operational_failure_cause(&report, operational_failure);
    let report_path = format!("{}/{report_id}.json", types::REPORT_PATH);
    let ledger_path = format!("decomposition/reverify/remediation-{report_id}.json");
    let report_event = serde_json::json!({"report_id": report_id, "invocation_id": invocation_id,
        "report_digest": report_digest,
        "supersedes_id": prior.call.id, "outcome": outcome});
    let next_status = if all_accepted && reverify_paused {
        Some(RunStatus::Completed)
    } else if !operational_failure && outcome == "needs_remediation" {
        Some(RunStatus::Paused)
    } else {
        None
    };
    let ledger = failed_subjects.as_ref().map(|subjects| {
        serde_json::json!({
            "schema_version": 1, "report_id": report_id, "subjects": subjects,
        })
    });
    publish::final_report(
        &store,
        run_id,
        &run_dir,
        &_lease,
        invocation_generation,
        &invocation_status,
        &v2,
        &report_path,
        &report_bytes,
        &report_record,
        report_event_seq,
        report_event,
        ledger.as_ref().map(|value| (ledger_path.as_str(), value)),
        next_status,
    )?;
    if let Some(cause) = operational_cause {
        return Err(anyhow!(
            "reverify recorded operational_failure: {cause}; run reverify again"
        ));
    }
    let verdicts = report
        .subjects
        .iter()
        .map(|subject| format!("{}={}", subject.task_id, subject.verdict))
        .chain(
            report
                .checks
                .iter()
                .map(|check| format!("{}={}", check.check_id, check.verdict)),
        )
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "Reverify report: {report_id}\nOutcome: {outcome}\nSubjects: {verdicts}\nResume: archon workflow resume --live --yes {run_id}"
    ))
}
