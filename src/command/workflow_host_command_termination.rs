//! Platform-specific confinement and teardown of one host command.
//!
//! On Unix the command leads its own session (Issue 270). Its tree is
//! tracked by an `archon_shell::process_tree::Tracker`: the leader is pinned
//! at spawn, its group and session ids name members only while it is
//! unreaped, scans while it runs pin every descendant, and teardown kills
//! every pinned member still alive. The leader is NOT reaped when it exits:
//! its exit is observed with `waitid(WNOWAIT)`, the tree is torn down while
//! the unreaped leader still holds its pid (so no stranger can hold its
//! group or session id), and only then is it reaped.
//!
//! On Windows the command runs in a Job Object this supervisor owns
//! (Issue 273); teardown terminates the job and confirms that no process in
//! it is still active.
//!
//! Teardown never fails a call. A member that will not die, a leader that
//! cannot be reaped, or a tree that cannot be scanned is a [`Teardown::
//! Stalled`], which the supervisor turns into an operational, resumable
//! outcome and the executor into a paused run (Issue 270 round 3).
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(unix)]
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use archon_shell::process_tree::{Pinned, ReapToken, Tracker, lock_until};
#[cfg(unix)]
use archon_workflow::WorkflowResult;

#[cfg(unix)]
use super::CLEANUP_GRACE;
use super::REAP_DEADLINE;

/// How long one background scan may take; a scan past it is dropped whole.
#[cfg(unix)]
const SCAN_BUDGET: Duration = Duration::from_secs(2);
/// How often the leader's exit is polled (without reaping it).
#[cfg(unix)]
const EXIT_POLL: Duration = Duration::from_millis(20);

/// What teardown established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Teardown {
    /// No process of the tree is left.
    Confirmed,
    /// Teardown could not finish: `evidence` says why. `survivors` names the
    /// members still alive (pid, start time), or is `None` when they are not
    /// known: then a resume must refuse until someone verifies the tree.
    Stalled {
        evidence: String,
        survivors: Option<Vec<(u32, u64)>>,
    },
}

impl Teardown {
    /// A stall whose survivors are not known.
    pub(super) fn stalled(evidence: impl Into<String>) -> Self {
        Self::Stalled {
            evidence: evidence.into(),
            survivors: None,
        }
    }

    /// This outcome, with `evidence` added as a further cause of a stall
    /// whose survivors are not known.
    pub(super) fn and_stalled(self, evidence: impl Into<String>) -> Self {
        let evidence = evidence.into();
        match self {
            Self::Confirmed => Self::stalled(evidence),
            Self::Stalled {
                evidence: earlier, ..
            } => Self::stalled(format!("{earlier}; {evidence}")),
        }
    }
}

/// Everything that confines one spawned host command.
#[derive(Clone)]
pub(super) struct Tree {
    leader: Option<u32>,
    pub(super) evidence: Option<super::super::workflow_host_command_groups::GroupEvidence>,
    #[cfg(unix)]
    pub(super) tracker: Arc<Mutex<Tracker>>,
    #[cfg(unix)]
    reaped: ReapToken,
    /// Set once supervision stopped: a background scan still running then
    /// absorbs nothing.
    #[cfg(unix)]
    abandoned: Arc<AtomicBool>,
    #[cfg(windows)]
    job: Option<std::sync::Arc<archon_shell::job_object::Job>>,
}

impl Tree {
    /// The pid of the command, which leads its group (and, on Unix, its
    /// session).
    pub(super) fn leader(&self) -> Option<u32> {
        self.leader
    }

    /// The name of the command's Job Object, which a resume can probe.
    pub(super) fn job_name(&self) -> Option<&str> {
        #[cfg(windows)]
        {
            self.job.as_deref().and_then(|job| job.name())
        }
        #[cfg(not(windows))]
        {
            None
        }
    }
}

#[cfg(windows)]
#[path = "workflow_host_command_termination_windows.rs"]
mod windows;
#[cfg(windows)]
pub(super) use windows::{
    confine, kill_blocking, leader_exit, terminate_and_reap, terminate_completed_group,
};

/// Confines a child spawned as a session leader (`pre_exec` in the
/// supervisor): the leader is pinned, and its group and session ids select
/// members while it is unreaped.
#[cfg(unix)]
pub(super) fn confine(child: &mut tokio::process::Child) -> WorkflowResult<Tree> {
    let leader = child.id();
    let tracker = leader.map_or_else(Tracker::default, |pid| {
        // A leader that already exited is still our unreaped child: its pid
        // is ours, and only its start time is unknown.
        let start = archon_shell::process_tree::start_of(pid).unwrap_or_default();
        Tracker::new(Pinned { pid, start }, vec![pid], vec![pid])
    });
    Ok(Tree {
        leader,
        evidence: None,
        reaped: tracker.reap_token(),
        tracker: Arc::new(Mutex::new(tracker)),
        abandoned: Arc::new(AtomicBool::new(false)),
    })
}

/// Resolves when the leader exits, without reaping it.
#[cfg(unix)]
pub(super) async fn leader_exit(child: &mut tokio::process::Child) -> std::io::Result<()> {
    let Some(pid) = child.id() else {
        return Ok(());
    };
    loop {
        if archon_shell::process_tree::exited(pid)? {
            return Ok(());
        }
        tokio::time::sleep(EXIT_POLL).await;
    }
}

#[cfg(unix)]
impl Tree {
    /// Start one scan on a blocking thread and return at once: the
    /// supervisor's select keeps watching timeouts and control meanwhile.
    /// The scan reads the table without the tracker lock, within its own
    /// budget, and takes the lock only to absorb a complete table.
    pub(super) fn spawn_refresh(&self) -> Option<tokio::task::JoinHandle<()>> {
        #[cfg(test)]
        if SCANS_PAUSED.with(std::cell::Cell::get) {
            return None;
        }
        let tracker = self.tracker.clone();
        let abandoned = self.abandoned.clone();
        Some(tokio::task::spawn_blocking(move || {
            let deadline = Instant::now() + SCAN_BUDGET;
            let Ok(table) = archon_shell::process_tree::snapshot_until(deadline) else {
                return;
            };
            if abandoned.load(Ordering::SeqCst) {
                return;
            }
            if let Ok(mut tracker) = lock_until(&tracker, deadline)
                && !abandoned.load(Ordering::SeqCst)
            {
                let _ = tracker.absorb_until(&table, deadline);
            }
        }))
    }

    /// The leader is reaped: its pid may be reused, so its group and session
    /// ids select nothing any more.
    pub(super) fn leader_reaped(&self) {
        self.reaped.mark_reaped();
    }

    async fn run(
        &self,
        bound: Duration,
        work: impl FnOnce(&mut Tracker, Instant) -> Teardown + Send + 'static,
    ) -> Teardown {
        // Disable before the first termination scan, including the TERM
        // grace. Outstanding scanners recheck this after acquiring the lock.
        self.abandoned.store(true, Ordering::SeqCst);
        let evidence = self.evidence.clone();
        let tracker = self.tracker.clone();
        let deadline = Instant::now() + bound;
        let task = tokio::task::spawn_blocking(move || {
            let persistence = evidence
                .as_ref()
                .and_then(|evidence| evidence.begin().err());
            let teardown = match lock_until(&tracker, deadline) {
                Ok(mut tracker) => work(&mut tracker, deadline),
                Err(error) => {
                    Teardown::stalled(format!("host command tree tracker unavailable: {error}"))
                }
            };
            match persistence {
                Some(error) => {
                    teardown.and_stalled(format!("recording teardown identities failed: {error}"))
                }
                None => teardown,
            }
        });
        match tokio::time::timeout(bound, task).await {
            Ok(Ok(teardown)) => teardown,
            Ok(Err(error)) => Teardown::stalled(format!("teardown task failed: {error}")),
            Err(_) => {
                Teardown::stalled("host command teardown deadline exceeded; survivors unknown")
            }
        }
    }
}

#[cfg(all(test, unix))]
thread_local! {
    /// Test seam: while set on the thread that drives a supervisor, its
    /// periodic scans do not run, so a test can act before any scan without
    /// racing the scan interval.
    pub(crate) static SCANS_PAUSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Kill every member of the tree within `bound`.
#[cfg(unix)]
fn kill_tracked(
    tracker: &mut Tracker,
    bound: Duration,
    evidence: Option<&super::super::workflow_host_command_groups::GroupEvidence>,
) -> Teardown {
    let killed = tracker.kill(bound);
    if let Ok(survivors) = &killed
        && let Some(evidence) = evidence
    {
        let pins: Vec<_> = survivors.iter().map(|pin| (pin.pid, pin.start)).collect();
        if let Err(error) = evidence.complete(&pins) {
            return Teardown::stalled(format!("recording teardown identities failed: {error}"));
        }
    }
    match killed {
        Ok(survivors) if survivors.is_empty() => Teardown::Confirmed,
        Ok(survivors) => Teardown::Stalled {
            evidence: format!(
                "host command process tree still has live members after termination: {}",
                survivors
                    .iter()
                    .map(|p| p.pid.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            survivors: Some(survivors.iter().map(|p| (p.pid, p.start)).collect()),
        },
        Err(error) => Teardown::stalled(format!("host command process tree scan failed: {error}")),
    }
}

/// Asks the tree to stop, kills what is left after the grace (every member
/// the SIGTERM round found stays pinned), and reaps the direct child within
/// [`REAP_DEADLINE`]. The leader is still unreaped during the kill.
#[cfg(unix)]
pub(super) async fn terminate_and_reap(child: &mut tokio::process::Child, tree: &Tree) -> Teardown {
    let _ = tree
        .run(SCAN_BUDGET, |tracker, deadline| {
            let _ = tracker.signal(libc::SIGTERM, deadline);
            Teardown::Confirmed
        })
        .await;
    tokio::time::sleep(CLEANUP_GRACE).await;
    let evidence = tree.evidence.clone();
    let teardown = tree
        .run(REAP_DEADLINE, move |tracker, deadline| {
            kill_tracked(
                tracker,
                deadline.saturating_duration_since(Instant::now()),
                evidence.as_ref(),
            )
        })
        .await;
    match reap(child).await {
        Ok(_) => teardown,
        Err(evidence) => teardown.and_stalled(evidence),
    }
}

/// After the leader exited on its own, before it is reaped: kill whatever
/// it left behind, including members that escaped its group and session.
#[cfg(unix)]
pub(super) async fn terminate_completed_group(tree: &Tree) -> Teardown {
    let evidence = tree.evidence.clone();
    tree.run(REAP_DEADLINE, move |tracker, deadline| {
        kill_tracked(
            tracker,
            deadline.saturating_duration_since(Instant::now()),
            evidence.as_ref(),
        )
    })
    .await
}

/// The teardown of a supervisor that stopped without settling (dropped or
/// returned early). Runs on a dedicated thread, never on the runtime. The
/// child handle remains owned by the cleanup thread, so the original
/// selectors stay valid until scan/kill finishes. The tracker lock is only tried, within the bound; if a scan still holds it,
/// the survivors are unknown.
#[cfg(unix)]
pub(super) fn kill_blocking(tree: &Tree, leader_unreaped: bool) -> Teardown {
    tree.abandoned.store(true, Ordering::SeqCst);
    if !leader_unreaped {
        tree.leader_reaped();
    }
    let persistence = tree
        .evidence
        .as_ref()
        .and_then(|evidence| evidence.begin().err());
    let deadline = Instant::now() + REAP_DEADLINE;
    let teardown = match lock_until(&tree.tracker, deadline) {
        Ok(mut tracker) => kill_tracked(
            &mut tracker,
            deadline.saturating_duration_since(Instant::now()),
            tree.evidence.as_ref(),
        ),
        Err(error) => Teardown::stalled(format!(
            "the tree was unavailable when supervision stopped: {error}"
        )),
    };
    match persistence {
        Some(error) => {
            teardown.and_stalled(format!("recording teardown identities failed: {error}"))
        }
        None => teardown,
    }
}

/// Reap the direct child within [`REAP_DEADLINE`]; the error is the evidence
/// of a stall.
pub(super) async fn reap(
    child: &mut tokio::process::Child,
) -> Result<std::process::ExitStatus, String> {
    match tokio::time::timeout(REAP_DEADLINE, child.wait()).await {
        Ok(Ok(status)) => Ok(status),
        Ok(Err(error)) => Err(format!("host command process reap failed: {error}")),
        Err(_) => Err("host command process reap exceeded cleanup deadline".to_string()),
    }
}
