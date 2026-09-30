// WorkflowScriptHost: a call the host process never lived to record.
//
// Issue-213 C5. A pause or cancel leaves an interrupted record, but a host
// killed outright (SIGKILL, a crash, a closed terminal) runs no code at all,
// so its in-flight call left nothing: no record, no event, no trace of what it
// had been doing. So a durable marker is written just before a call is
// dispatched and removed once its record is on disk; a marker still present
// when the run next starts names a call its host died under, and that start
// records it (orphan detection).
//
// Also where a call's dispatched agent sessions are collected (#215): the
// live client notes each session it mints under the call id it served.
use super::*;

/// What is known about a call at the moment it is dispatched.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct InflightMarker {
    call: WorkflowV2HostCall,
    attempt: u32,
    input_hash: String,
    started_at: String,
    #[serde(default)]
    depends_on: Vec<String>,
    /// The host process that dispatched it.
    #[serde(default)]
    host_pid: u32,
}

/// The reason an orphaned call's record gives.
pub(super) const ORPHANED_REASON: &str = "host_process_ended";

fn marker_name(call_id: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in call_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}.json")
}

/// Whether another live process wrote the marker. This process's own pid
/// never counts: a marker it finds at start is from an earlier run in it.
fn host_alive(pid: u32) -> bool {
    if pid == 0 || pid == std::process::id() {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only probes whether the process exists; EPERM
        // means it exists and belongs to someone else.
        let probed = unsafe { libc::kill(pid as libc::pid_t, 0) };
        probed == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

impl WorkflowScriptHost {
    fn inflight_dir(&self) -> std::path::PathBuf {
        self.runner.v2_store.root().join("inflight")
    }

    /// Every agent session dispatched for `call_id` or a call nested under
    /// it, taken so the next call cannot claim them.
    pub(super) fn take_call_sessions(&self, call_id: &str) -> Vec<String> {
        super::super::super::workflow_live_v2_client::call_sessions::take_sessions(
            &self.runner.run_id,
            call_id,
        )
    }

    /// Best effort: a marker that cannot be written costs only the orphan
    /// record a later start would have made, never the call.
    pub(super) fn mark_inflight(
        &self,
        execution: &WorkflowV2CallExecution,
        attempt: u32,
        input_hash: &str,
    ) {
        let marker = InflightMarker {
            call: execution.call.clone(),
            attempt,
            input_hash: input_hash.to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            depends_on: execution.depends_on.clone(),
            host_pid: std::process::id(),
        };
        let dir = self.inflight_dir();
        let path = dir.join(marker_name(&execution.call.id));
        let written = std::fs::create_dir_all(&dir)
            .and_then(|()| serde_json::to_vec(&marker).map_err(std::io::Error::other))
            .and_then(|bytes| {
                let staged = path.with_extension("json.tmp");
                std::fs::write(&staged, bytes)?;
                std::fs::rename(&staged, &path)
            });
        if let Err(error) = written {
            tracing::warn!(call_id = %execution.call.id, %error, "in-flight marker not written");
        }
    }

    pub(super) fn clear_inflight(&self, call_id: &str) {
        let _ = std::fs::remove_file(self.inflight_dir().join(marker_name(call_id)));
    }

    /// Record every call a previous host process died under. A call answered
    /// after its marker was written (the record finished later) only loses
    /// its stale marker; an earlier record of the same call is never
    /// overwritten, so no finished work is replaced by an orphan note.
    pub(crate) fn record_orphaned_calls(&self) {
        let Ok(entries) = std::fs::read_dir(self.inflight_dir()) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let marker = std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<InflightMarker>(&bytes).ok());
            let Some(marker) = marker else {
                let _ = std::fs::remove_file(&path);
                continue;
            };
            // A host still alive owns its call; only a dead one's is orphaned.
            if host_alive(marker.host_pid) {
                continue;
            }
            // The marker goes only once its orphan is recorded: a failed
            // write leaves it for the next start (Batch O review).
            match self.record_orphan(&marker) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&path);
                }
                Err(error) => {
                    tracing::warn!(call_id = %marker.call.id, %error, "orphaned call not recorded");
                }
            }
        }
    }

    fn record_orphan(&self, marker: &InflightMarker) -> archon_workflow::WorkflowResult<()> {
        let call_id = &marker.call.id;
        let existing = self.runner.v2_store.load_call_record(call_id)?;
        // A record the dead host left `Running` is no answer: the orphan
        // replaces it. Any other record is kept (an earlier answer is never
        // replaced by an orphan note).
        let answered = existing
            .as_ref()
            .is_some_and(|record| record.status != WorkflowV2Status::Running);
        if answered {
            // Answered after the marker, or an earlier answer that must not
            // be replaced: either way the run log still names the orphan.
            self.emit_v2_event(
                WorkflowEventKind::StageStalled,
                serde_json::json!({
                    "event": "call_orphaned",
                    "call_id": call_id,
                    "reason": ORPHANED_REASON,
                    "started_at": marker.started_at,
                    "record_kept": true,
                }),
            );
            return Ok(());
        }
        let summary = format!(
            "workflow v2 call '{call_id}' was in flight when its host process (pid {}) ended without recording it; dispatched at {}",
            marker.host_pid, marker.started_at
        );
        let result = WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: summary.clone(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Blocker,
                summary,
            )],
            data: serde_json::json!({
                "call_id": call_id,
                "interrupted": ORPHANED_REASON,
                "started_at": marker.started_at,
                "host_pid": marker.host_pid,
            }),
            ..WorkflowV2Result::default()
        };
        let record = WorkflowV2CallRecord::new(
            self.runner.v2_store.run_id(),
            marker.call.clone(),
            marker.attempt,
            marker.input_hash.clone(),
            result,
            marker.depends_on.clone(),
        )
        .with_scaffold_hash(Some(self.scaffold_hash.clone()));
        self.runner.v2_store.save_call_record(&record)?;
        self.emit_call_finished_event(&record);
        Ok(())
    }
}

#[cfg(test)]
#[path = "workflow_live_v2_script_host_inflight_tests.rs"]
mod workflow_live_v2_script_host_inflight_tests;
