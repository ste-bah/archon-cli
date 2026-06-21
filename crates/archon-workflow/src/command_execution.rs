use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::acceptance::VerifyCommandReport;
use crate::error::{WorkflowError, WorkflowResult};
use crate::events::{WorkflowEventKind, WorkflowEventLog};
use crate::run::WorkflowRun;
use crate::spec::StageSpec;
use crate::store::{WorkflowStore, safe_path_component};
use crate::work_unit_gate;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CommandExecutionRecord {
    schema: String,
    run_id: String,
    stage_id: String,
    attempt_id: String,
    command_id: String,
    role: String,
    command: String,
    cwd: String,
    process_group: Option<u32>,
    started_at: String,
    last_output_at: Option<String>,
    last_progress_at: Option<String>,
    progress_class: String,
    status: String,
    exit_status: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CancellationSummary {
    pub scanned_records: usize,
    pub cancelled_process_groups: Vec<u32>,
    pub failed_process_groups: Vec<u32>,
    pub evidence_path: Option<String>,
}

pub(crate) fn run_verify_command(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
    cwd: &Path,
    command: Option<&str>,
) -> WorkflowResult<Option<VerifyCommandReport>> {
    let Some(command) = command.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if contains_unresolved_workflow_template(command) {
        return Err(WorkflowError::StageFailed(format!(
            "verify_command contains unresolved workflow template `{command}`"
        )));
    }
    let started_at = Utc::now();
    let mut record = new_record(run, stage, cwd, command, started_at);
    write_record(store, &record)?;
    let mut child = spawn_shell(command, cwd).map_err(|err| {
        record.status = "failed_to_start".into();
        record.last_progress_at = Some(Utc::now().to_rfc3339());
        let _ = write_record(store, &record);
        WorkflowError::StageFailed(format!("verify_command failed to launch: {err}"))
    })?;
    record.process_group = process_group(&child);
    write_record(store, &record)?;

    let stdout = child.stdout.take().map(read_stream);
    let stderr = child.stderr.take().map(read_stream);
    let mut stall_emitted = false;
    let stall_after = stall_after(stage);
    let status = loop {
        match child.try_wait().map_err(|err| {
            WorkflowError::StageFailed(format!("verify_command wait failed: {err}"))
        })? {
            Some(status) => break status,
            None => {
                if !stall_emitted
                    && Utc::now().signed_duration_since(started_at).num_seconds()
                        >= stall_after as i64
                {
                    record.status = "stalled".into();
                    record.progress_class = "no_progress".into();
                    record.last_progress_at = Some(Utc::now().to_rfc3339());
                    write_record(store, &record)?;
                    emit_stall_event(store, &record)?;
                    stall_emitted = true;
                }
                thread::sleep(Duration::from_millis(250));
            }
        }
    };
    let finished_at = Utc::now();
    let stdout = join_stream(stdout)?;
    let stderr = join_stream(stderr)?;
    let report = VerifyCommandReport {
        command: command.to_string(),
        exit_code: status.code(),
        stdout,
        stderr,
    };
    let externally_cancelled =
        read_record(store, &record.run_id, &record.stage_id, &record.command_id)
            .ok()
            .is_some_and(|current| {
                matches!(current.status.as_str(), "cancelled" | "cancel_failed")
            });
    record.status = if externally_cancelled {
        "cancelled".into()
    } else if report.success() {
        "completed".into()
    } else {
        "failed".into()
    };
    record.exit_status = status.code();
    record.last_progress_at = Some(finished_at.to_rfc3339());
    record.progress_class =
        progress_class(command, &format!("{}\n{}", report.stdout, report.stderr));
    if !report.stdout.is_empty() || !report.stderr.is_empty() {
        record.last_output_at = Some(finished_at.to_rfc3339());
    }
    write_record(store, &record)?;
    Ok(Some(report))
}

fn contains_unresolved_workflow_template(command: &str) -> bool {
    let mut rest = command;
    while let Some(start) = rest.find("${") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find('}') else {
            return false;
        };
        let inner = rest[..end].trim();
        if inner.contains('.') {
            return true;
        }
        rest = &rest[end + 1..];
    }
    false
}

pub(crate) fn cancel_running_commands(
    store: &WorkflowStore,
    run: &WorkflowRun,
) -> WorkflowResult<CancellationSummary> {
    let mut scanned = 0usize;
    let mut cancelled = Vec::new();
    let mut failed = Vec::new();
    for path in command_record_paths(store, &run.id)? {
        let raw = fs::read_to_string(&path).map_err(|err| WorkflowError::io(&path, err))?;
        let mut record: CommandExecutionRecord = serde_json::from_str(&raw)?;
        scanned += 1;
        if !matches!(record.status.as_str(), "running" | "stalled") {
            continue;
        }
        let Some(pgid) = record.process_group else {
            record.status = "cancel_failed".into();
            write_record(store, &record)?;
            continue;
        };
        if terminate_process_group(pgid) {
            record.status = "cancelled".into();
            record.last_progress_at = Some(Utc::now().to_rfc3339());
            record.exit_status = None;
            write_record(store, &record)?;
            cancelled.push(pgid);
        } else {
            record.status = "cancel_failed".into();
            record.last_progress_at = Some(Utc::now().to_rfc3339());
            write_record(store, &record)?;
            failed.push(pgid);
        }
    }
    let evidence_path = write_cancellation_evidence(store, run, scanned, &cancelled, &failed)?;
    Ok(CancellationSummary {
        scanned_records: scanned,
        cancelled_process_groups: cancelled,
        failed_process_groups: failed,
        evidence_path,
    })
}

fn new_record(
    run: &WorkflowRun,
    stage: &StageSpec,
    cwd: &Path,
    command: &str,
    started_at: DateTime<Utc>,
) -> CommandExecutionRecord {
    CommandExecutionRecord {
        schema: "archon.workflow.command_execution.v1".into(),
        run_id: run.id.clone(),
        stage_id: stage.id.clone(),
        attempt_id: work_unit_gate::attempt_id(run, stage),
        command_id: command_id(run, stage),
        role: "verification".into(),
        command: command.to_string(),
        cwd: cwd.display().to_string(),
        process_group: None,
        started_at: started_at.to_rfc3339(),
        last_output_at: None,
        last_progress_at: Some(started_at.to_rfc3339()),
        progress_class: progress_class(command, ""),
        status: "running".into(),
        exit_status: None,
    }
}

fn write_record(store: &WorkflowStore, record: &CommandExecutionRecord) -> WorkflowResult<()> {
    store.write_run_json(
        &record.run_id,
        command_record_path(&record.stage_id, &record.command_id),
        record,
    )
}

fn read_record(
    store: &WorkflowStore,
    run_id: &str,
    stage_id: &str,
    command_id: &str,
) -> WorkflowResult<CommandExecutionRecord> {
    let rel = command_record_path(stage_id, command_id);
    let path = store.run_dir(run_id).join(rel);
    let raw = fs::read_to_string(&path).map_err(|err| WorkflowError::io(&path, err))?;
    serde_json::from_str(&raw).map_err(WorkflowError::from)
}

fn command_id(run: &WorkflowRun, stage: &StageSpec) -> String {
    let attempt = run
        .stages
        .get(&stage.id)
        .map(|state| state.attempt.max(1))
        .unwrap_or(1);
    format!("cmd-{attempt:04}")
}

fn command_record_path(stage_id: &str, command_id: &str) -> PathBuf {
    PathBuf::from("command-executions")
        .join(safe_path_component(stage_id))
        .join(format!("{}.json", safe_path_component(command_id)))
}

fn command_record_paths(store: &WorkflowStore, run_id: &str) -> WorkflowResult<Vec<PathBuf>> {
    let root = store.run_dir(run_id).join("command-executions");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for stage in fs::read_dir(&root).map_err(|err| WorkflowError::io(&root, err))? {
        let stage = stage.map_err(|err| WorkflowError::io(&root, err))?;
        if !stage.path().is_dir() {
            continue;
        }
        for entry in fs::read_dir(stage.path()).map_err(|err| WorkflowError::io(&root, err))? {
            let entry = entry.map_err(|err| WorkflowError::io(&root, err))?;
            if entry.path().extension().and_then(|ext| ext.to_str()) == Some("json") {
                paths.push(entry.path());
            }
        }
    }
    Ok(paths)
}

fn spawn_shell(command: &str, cwd: &Path) -> std::io::Result<std::process::Child> {
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some((key, value)) = crate::cargo_target_env::guarded_cargo_target_env(command, cwd) {
        cmd.env(key, value);
    }
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    cmd.spawn()
}

fn process_group(child: &std::process::Child) -> Option<u32> {
    #[cfg(unix)]
    {
        Some(child.id())
    }
    #[cfg(not(unix))]
    {
        let _ = child;
        None
    }
}

fn read_stream(mut stream: impl Read + Send + 'static) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        out
    })
}

fn join_stream(handle: Option<thread::JoinHandle<String>>) -> WorkflowResult<String> {
    handle
        .map(|handle| {
            handle.join().map_err(|_| {
                WorkflowError::StageFailed("verify_command output reader panicked".into())
            })
        })
        .transpose()
        .map(|value| value.unwrap_or_default())
}

fn emit_stall_event(store: &WorkflowStore, record: &CommandExecutionRecord) -> WorkflowResult<()> {
    let seq = store.next_event_seq(&record.run_id)?;
    WorkflowEventLog::new(store.clone()).emit(
        &record.run_id,
        seq,
        WorkflowEventKind::StageStalled,
        json!({
            "stage": record.stage_id,
            "command_id": record.command_id,
            "role": record.role,
            "progress_class": record.progress_class,
            "status": "stalled",
            "command_execution_record": command_record_path(&record.stage_id, &record.command_id),
        }),
    )?;
    Ok(())
}

fn stall_after(stage: &StageSpec) -> u64 {
    stage
        .extra
        .get("command_stall_after_secs")
        .or_else(|| stage.input.get("command_stall_after_secs"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(300)
}

fn progress_class(command: &str, output: &str) -> String {
    let text = format!("{command} {output}").to_ascii_lowercase();
    let class = if text.contains("compiling") || text.contains("checking") {
        "compiling"
    } else if text.contains("running") && text.contains("test") {
        "running_tests"
    } else if text.contains("--list") || text.contains("list-tests") {
        "listing_tests"
    } else if text.contains("collect-only") {
        "collecting_tests"
    } else if text.contains("install") {
        "installing_dependencies"
    } else if text.contains("materializ") {
        "materializing_artifact"
    } else if text.contains("validat") {
        "validating_artifact"
    } else {
        "unknown_progress"
    };
    class.into()
}

#[cfg(unix)]
fn terminate_process_group(pgid: u32) -> bool {
    let group = -(pgid as i32);
    unsafe {
        let _ = libc::kill(group, libc::SIGTERM);
    }
    thread::sleep(Duration::from_millis(250));
    if !process_group_alive(pgid) {
        return true;
    }
    unsafe {
        let _ = libc::kill(group, libc::SIGKILL);
    }
    thread::sleep(Duration::from_millis(100));
    !process_group_alive(pgid)
}

#[cfg(unix)]
fn process_group_alive(pgid: u32) -> bool {
    unsafe { libc::kill(-(pgid as i32), 0) == 0 }
}

#[cfg(not(unix))]
fn terminate_process_group(_pgid: u32) -> bool {
    false
}

fn write_cancellation_evidence(
    store: &WorkflowStore,
    run: &WorkflowRun,
    scanned: usize,
    cancelled: &[u32],
    failed: &[u32],
) -> WorkflowResult<Option<String>> {
    if scanned == 0 {
        return Ok(None);
    }
    let rel = PathBuf::from("command-cancellations")
        .join(format!("cancel-{}.json", Utc::now().timestamp_millis()));
    store.write_run_json(
        &run.id,
        &rel,
        &json!({
            "schema": "archon.workflow.command_cancellation.v1",
            "run_id": run.id,
            "scanned_records": scanned,
            "cancelled_process_groups": cancelled,
            "failed_process_groups": failed,
            "created_at": Utc::now().to_rfc3339(),
        }),
    )?;
    Ok(Some(rel.display().to_string()))
}
