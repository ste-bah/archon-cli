//! Recovery of a fixed run whose executor died while the run was `Running`
//! (Issue 251).
//!
//! A kill (SIGKILL, OOM, reboot, a closed terminal group) runs no code, so the
//! run stays `Running` with orphaned in-flight markers. The caller holds the
//! executor lease when it calls here: the kernel lock that the lease is proves
//! that no live process executes the run. The recovery records that fact and
//! its evidence as a `stale_owner_recovered` event and moves the run to
//! `Paused`, which is the state the normal resume path accepts. It never acts
//! without the lease, so a live owner is never displaced.

use std::path::Path;

use archon_workflow::{RunStatus, WorkflowEventKind, WorkflowEventLog, WorkflowStore};

use super::workflow_executor_lease::{ExecutionLease, LEASE, pid_running};

/// Where the v2 script host keeps one marker per call in flight.
const INFLIGHT_DIR: &str = "v2/inflight";

/// What the recovery found and recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaleOwnerRecovery {
    pub(crate) previous_pid: Option<u32>,
    pub(crate) orphaned_inflight_markers: usize,
}

impl StaleOwnerRecovery {
    pub(crate) fn summary(&self, run_id: &str) -> String {
        let pid = self
            .previous_pid
            .map_or_else(|| "unrecorded".to_string(), |pid| pid.to_string());
        format!(
            "Stale owner recovered: run {run_id} was Running but no live process held {LEASE} (previous executor pid {pid}, {} orphaned in-flight call(s)); recorded stale_owner_recovered and moved the run to Paused\n",
            self.orphaned_inflight_markers
        )
    }
}

/// Moves `run_id` from `Running` to `Paused` with a `stale_owner_recovered`
/// event, while `lease` proves that its executor is dead. Returns `None` and
/// changes nothing when the run is not `Running`.
pub(crate) fn recover_dead_owner(
    store: &WorkflowStore,
    run_id: &str,
    lease: &ExecutionLease,
    log_path: &Path,
) -> anyhow::Result<Option<StaleOwnerRecovery>> {
    let previous = lease.previous_holder().cloned();
    let (marker_count, host_pids) = inflight_evidence(&store.run_dir(run_id).join(INFLIGHT_DIR));
    let recovery = store.with_run_lock(run_id, |locked| {
        let mut run = locked.load_state(run_id)?;
        if run.status != RunStatus::Running {
            return Ok(None);
        }
        run.generation = run.generation.saturating_add(1);
        let detail = archon_workflow::events::sanitize_value(serde_json::json!({
            "event": "stale_owner_recovered",
            "previous_status": "running",
            "status": "paused",
            "generation": run.generation,
            "owner_lock": LEASE,
            "owner_lock_free": true,
            "previous_owner_pid": previous.as_ref().map(|holder| holder.pid),
            "previous_owner_acquired_at": previous.as_ref().map(|holder| holder.acquired_at.clone()),
            // Evidence only: a reused pid can be running and still not be
            // the owner; the free kernel lock is what proves the owner dead.
            "previous_owner_pid_running": previous.as_ref().and_then(|holder| pid_running(holder.pid)),
            "orphaned_inflight_markers": marker_count,
            "inflight_host_pids": host_pids,
            "recovered_by_pid": std::process::id(),
        }));
        // Recorded before the transition, so a run never leaves `Running`
        // without the record of why.
        let seq = locked.next_event_seq(run_id)?;
        WorkflowEventLog::new(locked.clone()).emit(
            run_id,
            seq,
            WorkflowEventKind::StaleOwnerRecovered,
            detail,
        )?;
        run.status = RunStatus::Paused;
        run.mark_updated();
        locked.save_state(&run)?;
        Ok(Some(seq))
    })?;
    let Some(seq) = recovery else {
        return Ok(None);
    };
    let previous_pid = previous.map(|holder| holder.pid);
    let line = format!(
        "event_id={seq} transition=stale_owner_recovered previous_pid={} orphaned_inflight={marker_count}",
        previous_pid.map_or_else(|| "unrecorded".to_string(), |pid| pid.to_string())
    );
    crate::command::workflow_decompose_log::append_nofollow_line(log_path, &line)?;
    Ok(Some(StaleOwnerRecovery {
        previous_pid,
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
