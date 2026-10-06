//! Recovery of a run whose executor died while the run was `Running`: a fixed
//! decomposition (Issue 251) or a generic V2 run (Issue 252).
//!
//! A kill (SIGKILL, OOM, reboot, a closed terminal group) runs no code, so the
//! run stays `Running` with orphaned in-flight markers. The caller holds the
//! executor lease when it calls here: the kernel lock that the lease is proves
//! that no live process executes the run. The caller has also refused while a
//! host-command process group that the dead executor started still runs. The
//! recovery records that fact and its evidence as a `stale_owner_recovered`
//! event and moves the run to `Paused`, which is the state the normal resume
//! path accepts. It never acts without the lease, so a live owner is never
//! displaced.

use std::path::Path;

use archon_workflow::process_liveness::process_alive;
use archon_workflow::{RunStatus, WorkflowEventKind, WorkflowEventLog, WorkflowStore};

use super::workflow_executor_lease::{ExecutionLease, LEASE};
use super::workflow_host_command_groups::HostCommandGroupRecord;

/// Where the v2 script host keeps one marker per call in flight.
const INFLIGHT_DIR: &str = "v2/inflight";

/// What the recovery found and recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaleOwnerRecovery {
    pub(crate) previous_pid: Option<u32>,
    pub(crate) owner_state: &'static str,
    pub(crate) orphaned_inflight_markers: usize,
}

impl StaleOwnerRecovery {
    pub(crate) fn summary(&self, run_id: &str) -> String {
        let owner = match (self.previous_pid, self.owner_state) {
            (Some(pid), "exited") => format!("previous executor pid {pid} has exited"),
            (Some(pid), "pid_reused") => format!(
                "previous executor pid {pid} now belongs to another process that does not hold the lease"
            ),
            (Some(pid), _) => format!("previous executor pid {pid}, state unknown"),
            (None, _) => "previous executor not recorded".to_string(),
        };
        format!(
            "Stale owner recovered: run {run_id} was Running but no live process held {LEASE} ({owner}; {} orphaned in-flight call(s)); recorded stale_owner_recovered and moved the run to Paused\n",
            self.orphaned_inflight_markers
        )
    }
}

/// How the previous executor's pid looks now. Evidence only: the OS can give
/// a dead owner's pid to an unrelated process, so a running pid never means
/// "owner".
fn owner_state(pid: Option<u32>) -> &'static str {
    match pid {
        None => "unrecorded",
        // The lock is free, so whatever runs under this pid is not the owner.
        Some(pid) if process_alive(pid) => "pid_reused",
        Some(_) => "exited",
    }
}

/// Moves `run_id` from `Running` to `Paused` with a `stale_owner_recovered`
/// event, while `lease` proves that its executor is dead. `ended_groups` are
/// the host-command groups the dead executor left that have since ended.
/// Returns `None` and changes nothing when the run is not `Running`.
pub(crate) fn recover_dead_owner(
    store: &WorkflowStore,
    run_id: &str,
    lease: &ExecutionLease,
    log_path: &Path,
    ended_groups: &[HostCommandGroupRecord],
) -> anyhow::Result<Option<StaleOwnerRecovery>> {
    recover(store, run_id, lease, Some(log_path), ended_groups)
}

/// [`recover_dead_owner`] for a generic V2 run, which keeps no fixed log: the
/// event is the whole record (Issue 252).
pub(crate) fn recover_dead_generic_owner(
    store: &WorkflowStore,
    run_id: &str,
    lease: &ExecutionLease,
    ended_groups: &[HostCommandGroupRecord],
) -> anyhow::Result<Option<StaleOwnerRecovery>> {
    recover(store, run_id, lease, None, ended_groups)
}

fn recover(
    store: &WorkflowStore,
    run_id: &str,
    lease: &ExecutionLease,
    log_path: Option<&Path>,
    ended_groups: &[HostCommandGroupRecord],
) -> anyhow::Result<Option<StaleOwnerRecovery>> {
    let previous = lease.previous_executor().cloned();
    let previous_pid = previous.as_ref().map(|holder| holder.pid);
    let state = owner_state(previous_pid);
    let (marker_count, host_pids) = inflight_evidence(&store.run_dir(run_id).join(INFLIGHT_DIR));
    let recovered = store.with_run_lock(run_id, |locked| {
        let mut run = locked.load_state(run_id)?;
        if run.status != RunStatus::Running {
            return Ok(false);
        }
        run.generation = run.generation.saturating_add(1);
        let detail = archon_workflow::events::sanitize_value(serde_json::json!({
            "event": "stale_owner_recovered",
            "previous_status": "running",
            "status": "paused",
            "generation": run.generation,
            "owner_lock": LEASE,
            "owner_lock_free": true,
            "owner_state": state,
            "previous_owner_pid": previous_pid,
            "previous_owner_acquired_at": previous.as_ref().map(|holder| holder.acquired_at.clone()),
            // Evidence only: a reused pid can be running and still not be
            // the owner; the free kernel lock is what proves the owner dead.
            "previous_owner_pid_running": previous_pid.map(process_alive),
            "orphaned_inflight_markers": marker_count,
            "inflight_host_pids": host_pids,
            "ended_host_command_groups": ended_groups,
            "recovered_by_pid": std::process::id(),
        }));
        // The event and the log line are written before the state commit,
        // so a run never leaves `Running` without the record of why. A
        // failure here leaves it `Running`, and the next resume records again.
        let seq = locked.next_event_seq(run_id)?;
        WorkflowEventLog::new(locked.clone()).emit(
            run_id,
            seq,
            WorkflowEventKind::StaleOwnerRecovered,
            detail,
        )?;
        if let Some(log_path) = log_path {
            let line = format!(
                "event_id={seq} transition=stale_owner_recovered previous_pid={} owner_state={state} orphaned_inflight={marker_count} ended_host_command_groups={}",
                previous_pid.map_or_else(|| "unrecorded".to_string(), |pid| pid.to_string()),
                ended_groups.len()
            );
            crate::command::workflow_decompose_log::append_nofollow_line(log_path, &line)?;
        }
        run.status = RunStatus::Paused;
        run.mark_updated();
        locked.save_state(&run)?;
        Ok(true)
    })?;
    Ok(recovered.then_some(StaleOwnerRecovery {
        previous_pid,
        owner_state: state,
        orphaned_inflight_markers: marker_count,
    }))
}

/// The number of in-flight markers and the distinct host pids they name.
fn inflight_evidence(dir: &Path) -> (usize, Vec<u32>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, Vec::new());
    };
    let mut count = 0;
    let mut pids = std::collections::BTreeSet::new();
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        count += 1;
        let pid = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|marker| marker.get("host_pid").and_then(serde_json::Value::as_u64))
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid != 0);
        pids.extend(pid);
    }
    (count, pids.into_iter().collect())
}
