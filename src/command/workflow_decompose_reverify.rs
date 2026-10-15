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

const REPORT_PATH: &str = "decomposition/reverify/reports";
const STATE_PATH: &str = "decomposition/state.json";

#[path = "workflow_decompose_reverify_persist_call.rs"]
mod persist;
pub(super) use persist::persist_call;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReverifyReport {
    schema_version: u32,
    id: String,
    supersedes_id: String,
    supersedes_digest: String,
    outcome: String,
    calls: Vec<String>,
    subjects: Vec<SubjectResult>,
    set_gates: Vec<GateResult>,
    frozen_gates: Vec<GateResult>,
    checks: Vec<acceptance::CheckResult>,
}

#[path = "workflow_decompose_reverify_acceptance.rs"]
mod acceptance;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SubjectResult {
    task_id: String,
    file_name: String,
    disk_digest: String,
    judged_digest: String,
    logic_version: Option<u32>,
    logic_digest: Option<String>,
    build: Option<String>,
    verdict: String,
    findings: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct GateResult {
    pub(super) command_id: String,
    pub(super) call_id: String,
    pub(super) logic_version: Option<u32>,
    pub(super) logic_digest: Option<String>,
    pub(super) build: Option<String>,
    pub(super) verdict: String,
    pub(super) findings: Vec<String>,
    pub(super) subjects: Vec<String>,
}

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
    if run.status != RunStatus::Completed {
        return Err(anyhow!("workflow {run_id} is not completed"));
    }
    let state_path = run_dir.join(STATE_PATH);
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
    let latest = crate::command::workflow_decompose_latest_report::latest_report(&store, run_id)?
        .ok_or_else(|| anyhow!("workflow {run_id} has no valid prior final report"))?;
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
    let executor =
        FixedHostCommandExecutor::new(catalog.clone(), base_context.clone(), run_dir.clone());
    let event_seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone()).emit(
        run_id,
        event_seq,
        WorkflowEventKind::DecompositionReverifyStarted,
        serde_json::json!({"invocation_id": format!("reverify-{event_seq}")}),
    )?;
    let mut call_ids = Vec::new();
    let mut subjects = Vec::new();
    let mut set_gates = Vec::new();
    let mut frozen_gates = Vec::new();
    let mut failed_subjects = Vec::new();
    let mut all_accepted = true;
    let mut operational_failure = false;

    let acceptance =
        acceptance::reverify_checks(&project_root, &prd_path, &task_root, config, env_vars).await?;
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
                    &v2,
                    run_id,
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
            &v2,
            run_id,
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
            &v2,
            run_id,
            run.generation,
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
            &v2,
            run_id,
            run.generation,
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
    let report_id = format!("reverify-{event_seq}-report");
    let report = ReverifyReport {
        schema_version: 1,
        id: report_id.clone(),
        supersedes_id: prior.call.id.clone(),
        supersedes_digest: prior_digest,
        outcome: outcome.to_string(),
        calls: call_ids.clone(),
        subjects,
        set_gates,
        frozen_gates,
        checks,
    };
    let report_bytes = serde_json::to_vec_pretty(&report)?;
    if v2.result_path(&report_id).exists()
        || run_dir
            .join(format!("{REPORT_PATH}/{report_id}.json"))
            .exists()
    {
        return Err(anyhow!("reverify report already exists: {report_id}"));
    }
    store.write_run_file(
        run_id,
        format!("{REPORT_PATH}/{report_id}.json"),
        &report_bytes,
    )?;
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
    v2.save_call_record(&report_record)?;
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone()).emit(
        run_id,
        seq,
        WorkflowEventKind::DecompositionReverified,
        serde_json::json!({"report_id": report_id, "report_digest": report_digest,
            "supersedes_id": prior.call.id, "outcome": outcome}),
    )?;
    if !operational_failure && !failed_subjects.is_empty() {
        let failed_subjects =
            crate::command::workflow_decompose_remediation::deduplicate_subjects(failed_subjects);
        let ledger_path = format!("decomposition/reverify/remediation-{report_id}.json");
        store.write_run_json(
            run_id,
            ledger_path,
            &serde_json::json!({
                "schema_version": 1, "report_id": report_id, "subjects": failed_subjects,
            }),
        )?;
        store.with_run_lock(run_id, |locked| {
            let mut latest = locked.load_state(run_id)?;
            latest.status = RunStatus::Paused;
            latest.updated_at = chrono::Utc::now();
            locked.save_state(&latest)
        })?;
    }
    if operational_failure {
        let causes = report
            .subjects
            .iter()
            .flat_map(|subject| subject.findings.iter())
            .chain(
                report
                    .set_gates
                    .iter()
                    .chain(&report.frozen_gates)
                    .flat_map(|gate| gate.findings.iter()),
            )
            .chain(report.checks.iter().flat_map(|check| check.findings.iter()))
            .cloned()
            .collect::<Vec<_>>();
        let cause = causes
            .first()
            .cloned()
            .unwrap_or_else(|| "gate gave no verdict".to_string());
        return Err(anyhow!(
            "reverify recorded operational_failure: {cause}; run reverify again"
        ));
    }
    if all_accepted {
        // Completion remains authoritative only when the current disk passes.
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
