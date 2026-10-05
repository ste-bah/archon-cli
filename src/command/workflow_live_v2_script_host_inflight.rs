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
//
// What the call's sessions were doing lives only in this process's memory
// (`archon_tools::session_progress`), and a killed process takes it with it.
// So while the call runs, its marker is rewritten every [`INFLIGHT_REFRESH`]
// with the sessions dispatched so far and their turns, last tool call and
// touched paths; the orphan record carries the last copy that reached disk.
use super::*;

/// How often a running call's marker is refreshed with its sessions' progress.
/// A killed host loses at most this much of what its call was doing.
const INFLIGHT_REFRESH: std::time::Duration = std::time::Duration::from_secs(15);

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
    /// The agent sessions dispatched for the call so far, in dispatch order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    agent_sessions: Vec<String>,
    /// Their progress when the marker was last written: `turns`,
    /// `last_tool_call`, `touched_paths` and one row per session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    progress: Option<serde_json::Value>,
}

impl InflightMarker {
    /// The marker for `execution` as of now, with what its sessions have
    /// reported so far. The sessions are read in place, never taken: the
    /// call's own record takes them when it is written.
    fn now(
        run_id: &str,
        execution: &WorkflowV2CallExecution,
        attempt: u32,
        input_hash: &str,
    ) -> Self {
        let agent_sessions =
            super::super::super::workflow_live_v2_client::call_sessions::peek_sessions(
                run_id,
                &execution.call.id,
            );
        let progress =
            super::workflow_live_v2_script_host_interrupt::interruption_progress(&agent_sessions);
        Self {
            call: execution.call.clone(),
            attempt,
            input_hash: input_hash.to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            depends_on: execution.depends_on.clone(),
            host_pid: std::process::id(),
            agent_sessions,
            progress: progress
                .as_object()
                .is_some_and(|fields| !fields.is_empty())
                .then_some(progress),
        }
    }
}

/// Write `marker` under `dir`, replacing any earlier copy atomically, so a
/// kill mid-write leaves the previous marker rather than a torn one.
fn write_marker(dir: &std::path::Path, marker: &InflightMarker) -> std::io::Result<()> {
    let path = dir.join(marker_name(&marker.call.id));
    std::fs::create_dir_all(dir)?;
    let bytes = serde_json::to_vec(marker).map_err(std::io::Error::other)?;
    let staged = path.with_extension("json.tmp");
    std::fs::write(&staged, bytes)?;
    std::fs::rename(&staged, &path)
}

/// Best effort: a marker that cannot be written costs only the orphan record
/// a later start would have made, never the call.
fn write_inflight(dir: &std::path::Path, marker: &InflightMarker) {
    if let Err(error) = write_marker(dir, marker) {
        tracing::warn!(call_id = %marker.call.id, %error, "in-flight marker not written");
    }
}

/// Run `work` under an in-flight marker in `dir`: written before it starts,
/// then rewritten with its sessions' progress every [`INFLIGHT_REFRESH`] until
/// it returns. Every copy keeps the dispatch time, so the orphan record still
/// says when the call started.
async fn refresh_while<T>(
    dir: &std::path::Path,
    run_id: &str,
    execution: &WorkflowV2CallExecution,
    attempt: u32,
    input_hash: &str,
    work: impl std::future::Future<Output = T>,
) -> T {
    let first = InflightMarker::now(run_id, execution, attempt, input_hash);
    write_inflight(dir, &first);
    let started_at = first.started_at;
    let mut tick = tokio::time::interval_at(
        tokio::time::Instant::now() + INFLIGHT_REFRESH,
        INFLIGHT_REFRESH,
    );
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // On the heap (#246): `tokio::pin!` would hold a second copy of the
    // call's whole dispatch future in this one.
    let mut work = Box::pin(work);
    loop {
        tokio::select! {
            biased;
            out = &mut work => return out,
            _ = tick.tick() => {
                let mut marker = InflightMarker::now(run_id, execution, attempt, input_hash);
                marker.started_at.clone_from(&started_at);
                write_inflight(dir, &marker);
            }
        }
    }
}

/// The reason an orphaned call's record gives.
pub(super) const ORPHANED_REASON: &str = "host_process_ended";

/// The reason a call's record gives when its started record was saved and
/// its dispatch never began (Issue 303).
pub(super) const UNSTARTED_REASON: &str = "dispatch_not_started";

/// Issue 303: the run stage of a call closed as interrupted leaves `Running`
/// for `NeedsReview`, as its record did. A call the script never reaches
/// again is in no summary, so no finalization would settle it.
pub(super) fn settle_interrupted_stage(
    locked: &WorkflowStore,
    run_id: &str,
    call_id: &str,
) -> archon_workflow::WorkflowResult<()> {
    let mut run = locked.load_state(run_id)?;
    let Some(stage) = run
        .stages
        .get_mut(call_id)
        .filter(|stage| stage.status == archon_workflow::StageStatus::Running)
    else {
        return Ok(());
    };
    stage.status = archon_workflow::StageStatus::NeedsReview;
    stage.completed_at.get_or_insert_with(chrono::Utc::now);
    run.mark_updated();
    locked.save_state_preserving_control(&run)
}

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

    /// Run `work`, the call's dispatch, under an in-flight marker kept
    /// current with its sessions' progress; see [`refresh_while`].
    pub(super) async fn refreshing_inflight<T>(
        &self,
        execution: &WorkflowV2CallExecution,
        attempt: u32,
        input_hash: &str,
        work: impl std::future::Future<Output = T>,
    ) -> T {
        refresh_while(
            &self.inflight_dir(),
            &self.runner.run_id,
            execution,
            attempt,
            input_hash,
            Box::pin(work),
        )
        .await
    }

    pub(super) fn clear_inflight(&self, call_id: &str) {
        self.forget_pending_call(call_id);
        let _ = std::fs::remove_file(self.inflight_dir().join(marker_name(call_id)));
    }

    /// Record every call a previous host process died under. A call answered
    /// after its marker was written (the record finished later) only loses
    /// its stale marker; an earlier record of the same call is never
    /// overwritten, so no finished work is replaced by an orphan note.
    /// Issue 303: then close what a host left `Running` with no marker.
    pub(crate) fn record_orphaned_calls(&self) {
        self.record_marked_orphans();
        self.close_unmarked_running_calls();
    }

    fn record_marked_orphans(&self) {
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

    /// Issue 303: close every `Running` record that has no in-flight marker.
    /// The marker is written before a call is dispatched, so such a record is
    /// a call whose started record was saved and whose dispatch never began;
    /// the executor that saved it is gone. Nothing ran, so it is closed as
    /// interrupted and the script dispatches the call again when it reaches
    /// it. A record with a marker is the marker's to settle (above).
    fn close_unmarked_running_calls(&self) {
        let records = match self.runner.v2_store.load_call_records() {
            Ok(records) => records,
            Err(error) => {
                tracing::warn!(%error, "running call records not read at start");
                return;
            }
        };
        for record in records {
            let marked = self
                .inflight_dir()
                .join(marker_name(&record.call.id))
                .exists();
            if record.status != WorkflowV2Status::Running || marked {
                continue;
            }
            if let Err(error) = self.record_unstarted(&record) {
                tracing::warn!(call_id = %record.call.id, %error, "unstarted call not closed");
            }
        }
    }

    fn record_unstarted(
        &self,
        running: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<()> {
        let call_id = &running.call.id;
        let summary = format!(
            "workflow v2 call '{call_id}' was {UNSTARTED_REASON}: its started record (attempt {}, {}) was saved, but no in-flight marker shows a dispatch and its executor ended",
            running.attempt, running.started_at
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
                "interrupted": UNSTARTED_REASON,
                "started_at": running.started_at,
                "inflight_marker": false,
            }),
            ..WorkflowV2Result::default()
        };
        let record = WorkflowV2CallRecord::new(
            self.runner.v2_store.run_id(),
            running.call.clone(),
            running.attempt,
            running.input_hash.clone(),
            result,
            running.depends_on.clone(),
        )
        .with_source_metadata(running.source_fingerprint.clone(), None)
        .with_scaffold_hash(Some(self.scaffold_hash.clone()));
        // Issue 291: only the run's owner closes it. The projection's UI
        // event is dropped: no script runs yet to show it to.
        self.with_owned_run_lock(|locked| {
            self.runner.v2_store.save_call_record(&record)?;
            crate::command::workflow_decompose_state::project_fixed_call(
                locked,
                &self.runner.run_id,
                &record,
                crate::command::workflow_decompose_state::FixedCallProjectionKind::Interrupted,
            )?;
            self.forget_completed_call(call_id)?;
            settle_interrupted_stage(locked, &self.runner.run_id, call_id)
        })?;
        self.emit_call_finished_event(&record);
        Ok(())
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
        let mut data = serde_json::json!({
            "call_id": call_id,
            "interrupted": ORPHANED_REASON,
            "started_at": marker.started_at,
            "host_pid": marker.host_pid,
        });
        // What the call's sessions had been doing, as of the last refresh
        // that reached disk before the host died.
        if let (Some(data), Some(progress)) = (
            data.as_object_mut(),
            marker
                .progress
                .as_ref()
                .and_then(serde_json::Value::as_object),
        ) {
            data.extend(progress.clone());
        }
        let result = WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: summary.clone(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Blocker,
                summary,
            )],
            data,
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
        .with_scaffold_hash(Some(self.scaffold_hash.clone()))
        .with_agent_sessions(marker.agent_sessions.clone());
        // Issue 291: only the run's owner records an orphan.
        self.with_owned_run_lock(|_| self.runner.v2_store.save_call_record(&record))?;
        self.emit_call_finished_event(&record);
        Ok(())
    }
}

#[cfg(test)]
#[path = "workflow_live_v2_script_host_inflight_tests.rs"]
mod workflow_live_v2_script_host_inflight_tests;
