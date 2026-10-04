//! The supervisor's hold on one command's tree and its resume record.
//!
//! Whatever way supervision ends - a select outcome, an early error after
//! confinement, or the supervisor future being dropped - the tree is torn
//! down and that teardown settled exactly once: confirmed, the resume record
//! is removed; stalled, the record is kept, with the survivors it can name,
//! so that a resume refuses while any of them still runs (Issue 270/273).
use super::super::workflow_host_command_groups::GroupRecordGuard;
use super::SupervisedProcessOutput;
use super::io::CapturedPipe;
use super::termination::{Teardown, Tree, kill_blocking};

pub(super) struct ProcessGroupGuard {
    pub(super) tree: Tree,
    #[cfg(unix)]
    pub(super) child: std::mem::ManuallyDrop<tokio::process::Child>,
    record: Option<GroupRecordGuard>,
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
            settled: false,
            reaped: false,
        }
    }

    pub(super) fn hold_record(&mut self, record: Option<GroupRecordGuard>) {
        self.record = record;
    }

    pub(super) fn reaped(&mut self) {
        self.reaped = true;
        self.tree.leader_reaped();
    }

    /// Settle what teardown established. Returns the evidence of a stall, or
    /// `None` when the tree is confirmed empty.
    pub(super) fn settle(&mut self, teardown: Teardown) -> Option<String> {
        self.settled = true;
        settle_record(self.record.take(), teardown)
    }
}

/// Confirmed: the record goes. Stalled: the record is kept, with the
/// survivors (or "unknown"), and the evidence is returned.
fn settle_record(record: Option<GroupRecordGuard>, teardown: Teardown) -> Option<String> {
    match teardown {
        Teardown::Confirmed => {
            drop(record);
            None
        }
        Teardown::Stalled {
            evidence,
            survivors,
        } => {
            if let Some(record) = record
                && let Some(error) = record.keep(survivors.as_deref())
            {
                return Some(format!("{evidence}; {error}"));
            }
            Some(evidence)
        }
    }
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
        // Shared, so a thread that cannot start still leaves the record to
        // settle here: kept, as "unknown survivors".
        let record = std::sync::Arc::new(std::sync::Mutex::new(self.record.take()));
        let held = record.clone();
        let reaped = self.reaped;
        let spawned = std::thread::Builder::new()
            .name("archon-host-command-teardown".into())
            .spawn(move || {
                let teardown = kill_blocking(&tree, !reaped);
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
                let record = held.lock().ok().and_then(|mut record| record.take());
                if let Some(evidence) = settle_record(record, teardown) {
                    tracing::warn!(%evidence, "host command teardown stalled after supervision stopped");
                }
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
        Some((out, err)) => ((out.bytes, out.total), (err.bytes, err.total)),
        None => ((Vec::new(), 0), (Vec::new(), 0)),
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
    }
}
