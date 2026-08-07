use std::fs;
use std::path::PathBuf;
// Only the unix process-group teardown sleeps between SIGTERM and SIGKILL; the
// `cfg(not(unix))` arm of `terminate_process_group` returns immediately. Left
// ungated these are dead on Windows, and `-D warnings` is a hard error there.
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{WorkflowError, WorkflowResult};
use crate::run::WorkflowRun;
use crate::store::{WorkflowStore, safe_path_component};

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

fn write_record(store: &WorkflowStore, record: &CommandExecutionRecord) -> WorkflowResult<()> {
    store.write_run_json(
        &record.run_id,
        command_record_path(&record.stage_id, &record.command_id),
        record,
    )
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
