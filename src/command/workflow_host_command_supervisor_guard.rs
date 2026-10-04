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
    record: Option<GroupRecordGuard>,
    settled: bool,
    /// Once the leader is reaped its pid may be reused, so the tree is no
    /// longer reached through it by ancestry.
    reaped: bool,
}

impl ProcessGroupGuard {
    pub(super) fn new(tree: Tree) -> Self {
        Self {
            tree,
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
        let record = self.record.take();
        match teardown {
            Teardown::Confirmed => {
                drop(record);
                None
            }
            Teardown::Stalled {
                evidence,
                survivors,
            } => {
                if let Some(record) = record {
                    record.keep(&survivors);
                }
                Some(evidence)
            }
        }
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if !self.settled {
            let teardown = kill_blocking(&self.tree, !self.reaped);
            if let Some(evidence) = self.settle(teardown) {
                tracing::warn!(%evidence, "host command teardown stalled after supervision stopped");
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
