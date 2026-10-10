//! The supervisor's hold on one command's tree and its resume record.
//!
//! Whatever way supervision ends - a select outcome, an early error after
//! confinement, or the supervisor future being dropped - the tree is torn
//! down and that teardown settled exactly once: confirmed, the resume record
//! is removed; stalled, the record is kept, with the survivors it can name,
//! so that a resume refuses while any of them still runs (Issue 270/273).
use super::super::workflow_host_command_groups::GroupRecordGuard;
use super::super::workflow_host_command_teardown_latch::TeardownToken;
use super::super::workflow_host_exit_drain::PendingWork;
use super::SupervisedProcessOutput;
use super::io::CapturedPipe;
use super::termination::{Teardown, Tree, kill_blocking};

pub(super) struct ProcessGroupGuard {
    pub(super) tree: Tree,
    #[cfg(unix)]
    pub(super) child: std::mem::ManuallyDrop<tokio::process::Child>,
    record: Option<GroupRecordGuard>,
    /// Reports to the call when the teardown is settled (#297 round 8).
    teardown: Option<TeardownToken>,
    settled: bool,
    /// Once the leader is reaped its pid may be reused, so the tree is no
    /// longer reached through it by ancestry.
    reaped: bool,
}

impl ProcessGroupGuard {
    pub(super) fn new(tree: Tree, #[cfg(unix)] child: tokio::process::Child) -> Self {
        Self {
            tree,
            #[cfg(unix)]
            child: std::mem::ManuallyDrop::new(child),
            record: None,
            teardown: None,
            settled: false,
            reaped: false,
        }
    }

    pub(super) fn track_teardown(&mut self, token: TeardownToken) {
        self.teardown = Some(token);
    }

    pub(super) fn hold_record(&mut self, record: Option<GroupRecordGuard>) {
        self.tree.evidence = record.as_ref().and_then(GroupRecordGuard::evidence);
        self.record = record;
    }

    pub(super) async fn install_recorder(&self) -> archon_workflow::WorkflowResult<()> {
        #[cfg(unix)]
        if let Some(evidence) = self.tree.evidence.clone() {
            let tracker = self.tree.tracker.clone();
            let progress = archon_shell::teardown_progress::Progress::new(super::REAP_DEADLINE);
            let work = tokio::task::spawn_blocking(move || {
                let deadline = std::time::Instant::now() + super::REAP_DEADLINE;
                archon_shell::process_tree::lock_until(&tracker, deadline)?
                    .set_recorder(Box::new(evidence))
            });
            progress
                .watch(work)
                .await
                .and_then(|result| result)
                .map_err(|error| {
                    archon_workflow::WorkflowError::HostOperational(format!(
                        "recording host command identities stalled: {error}"
                    ))
                })?;
        }
        Ok(())
    }

    pub(super) fn reaped(&mut self) {
        self.reaped = true;
        self.tree.leader_reaped();
    }

    /// Settle what teardown established. Returns the evidence of a stall, or
    /// `None` when the tree is confirmed empty.
    pub(super) async fn settle(
        &mut self,
        teardown: Teardown,
    ) -> archon_workflow::WorkflowResult<Option<String>> {
        self.settled = true;
        let record = self.record.take();
        // Ordinary queued cleanup stays supervised. A stalled checkpoint
        // returns HostOperational directly, so the executor pauses without
        // retrying or synchronously locking the owner registry on this thread.
        let files = record
            .as_ref()
            .map(|record| {
                record
                    .paths()
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" and ")
            })
            .unwrap_or_default();
        let progress = archon_shell::teardown_progress::Progress::new(super::REAP_DEADLINE);
        // The token moves with the work: it reports even if this future is
        // dropped while the blocking work still runs, and the exit waits for
        // it (#297 round 9).
        let token = self.teardown.take();
        let pending = PendingWork::begin();
        let work = tokio::task::spawn_blocking(move || {
            let (evidence, confirmed) = settle_record(record, teardown);
            report(token, evidence.is_none(), confirmed);
            drop(pending);
            evidence
        });
        progress.watch(work).await.map_err(|error| archon_workflow::WorkflowError::HostOperational(
            format!("settling host command survivor evidence stalled ({files}); repair the host I/O and recheck these records on resume: {error}")
        ))
    }
}

/// Confirmed: the record is returned, for [`report`] to remove. Stalled:
/// the record is kept, with the survivors (or "unknown"), and the evidence
/// is returned.
fn settle_record(
    record: Option<GroupRecordGuard>,
    teardown: Teardown,
) -> (Option<String>, Option<GroupRecordGuard>) {
    match teardown {
        Teardown::Confirmed => (None, record),
        Teardown::Stalled {
            evidence,
            survivors,
        } => {
            if let Some(record) = record
                && let Some(error) = record.keep(survivors.as_deref())
            {
                return (Some(format!("{evidence}; {error}")), None);
            }
            (Some(evidence), None)
        }
    }
}

/// Reports the settled teardown, which runs what waits for it (the
/// cancelled call's sealing), and only then removes a confirmed record: a
/// resume refuses while the record exists, so it cannot start while that
/// sealing still runs on the call's staging (#297 round 9).
fn report(token: Option<TeardownToken>, empty: bool, confirmed: Option<GroupRecordGuard>) {
    if let Some(token) = token {
        token.settled(empty);
    }
    drop(confirmed);
}

impl Drop for ProcessGroupGuard {
    /// Supervision stopped without settling (the future was dropped, or it
    /// returned early after confinement). The teardown can take seconds, so
    /// it runs on a dedicated thread, never on the async runtime, and the
    /// resume record stays until that teardown reports.
    fn drop(&mut self) {
        #[cfg(unix)]
        // SAFETY: this Drop is the only take; the field is not dropped again.
        // It keeps the leader unreaped until the worker has scanned its scope.
        let mut child = unsafe { std::mem::ManuallyDrop::take(&mut self.child) };
        if self.settled {
            return;
        }
        self.settled = true;
        let tree = self.tree.clone();
        // Registration already left incomplete evidence; checkpoint admission
        // and I/O belong to the watched worker, never this Drop caller.
        // Shared, so a thread that cannot start still leaves the record to
        // settle here: kept, as "unknown survivors".
        let record = std::sync::Arc::new(std::sync::Mutex::new(self.record.take()));
        let held = record.clone();
        let reaped = self.reaped;
        // Dropped unreported if the thread cannot start: then unconfirmed.
        let token = self.teardown.take();
        // The exit waits for this thread on every platform (#297 round 9).
        let pending = PendingWork::begin();
        let spawned = std::thread::Builder::new()
            .name("archon-host-command-teardown".into())
            .spawn(move || {
                let teardown = kill_blocking(&tree, !reaped);
                pending.progressed();
                #[cfg(unix)]
                let teardown = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(runtime) => match runtime.block_on(super::termination::reap(&mut child)) {
                        Ok(_) => teardown,
                        Err(evidence) => teardown.and_stalled(evidence),
                    },
                    Err(error) => teardown.and_stalled(format!("drop reap runtime unavailable: {error}")),
                };
                #[cfg(unix)]
                tree.leader_reaped();
                pending.progressed();
                let record = held.lock().ok().and_then(|mut record| record.take());
                let (evidence, confirmed) = settle_record(record, teardown);
                if let Some(evidence) = &evidence {
                    tracing::warn!(%evidence, "host command teardown stalled after supervision stopped");
                }
                pending.progressed();
                // Last: what waits for the teardown (the cancelled call's
                // sealing) runs once no process of the tree can write.
                report(token, evidence.is_none(), confirmed);
                drop(pending);
            });
        match spawned {
            Ok(handle) => {
                #[cfg(unix)]
                archon_shell::process_tree::register_cleanup(handle);
                #[cfg(not(unix))]
                drop(handle);
            }
            Err(error) => {
                if let Some(record) = record.lock().ok().and_then(|mut record| record.take()) {
                    record.keep(None);
                }
                tracing::error!(%error, "host command teardown thread could not start; survivors unknown");
            }
        }
    }
}

/// The resumable outcome of a stalled teardown (Issue 270 round 2): exit
/// status `EXIT_INCOMPLETE_RESUMABLE`, or still `timed_out` for a timeout,
/// with the evidence appended to stderr. The executor's operational path
/// retries or pauses on it; a stall never fails the call.
pub(super) fn stalled_output(
    evidence: &str,
    pipes: Option<(CapturedPipe, CapturedPipe)>,
    timed_out: bool,
) -> SupervisedProcessOutput {
    let (stdout, mut stderr) = match pipes {
        Some((out, err)) => (
            (out.bytes, out.total, out.retained, out.truncated, out.path),
            (err.bytes, err.total, err.retained, err.truncated, err.path),
        ),
        None => (
            (Vec::new(), 0, 0, false, None),
            (Vec::new(), 0, 0, false, None),
        ),
    };
    let note = format!("\nhost command teardown stalled: {evidence}\n");
    stderr.0.extend_from_slice(note.as_bytes());
    stderr.1 = stderr.1.saturating_add(note.len() as u64);
    SupervisedProcessOutput {
        exit_code: (!timed_out)
            .then_some(super::super::workflow_host_command_operational::EXIT_INCOMPLETE_RESUMABLE),
        timed_out,
        stdout: stdout.0,
        stderr: stderr.0,
        stdout_bytes: stdout.1,
        stderr_bytes: stderr.1,
        stdout_retained_bytes: stdout.2,
        stderr_retained_bytes: stderr.2.saturating_add(note.len() as u64),
        stdout_truncated: stdout.3,
        stderr_truncated: stderr.3,
        stdout_path: stdout.4,
        stderr_path: stderr.4,
    }
}
