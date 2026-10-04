//! Platform-specific confinement and teardown of one host command.
//!
//! On Unix the command leads its own session (Issue 270). A nested runner
//! may give each of its checks a process group of its own, and a descendant
//! may leave the session as well, so teardown works on the whole tree: the
//! command's group and session, every descendant by ancestry, and every
//! member a scan saw while the command ran (`archon_shell::process_tree::
//! Tracker`), each pinned by its start time so that a reused pid is never
//! signalled.
//!
//! On Windows the command runs in a Job Object this supervisor owns
//! (Issue 273); teardown terminates the job and confirms that no process in
//! it is still active.
//!
//! Teardown never fails a call. A member that will not die, a leader that
//! cannot be reaped, or a tree that cannot be scanned is a [`Teardown::
//! Stalled`], which the supervisor turns into an operational, resumable
//! outcome (Issue 270 round 2).
#[cfg(unix)]
use archon_workflow::WorkflowResult;

#[cfg(unix)]
use super::CLEANUP_GRACE;
use super::REAP_DEADLINE;

/// What teardown established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Teardown {
    /// No process of the tree is left.
    Confirmed,
    /// Teardown could not finish: `evidence` says why, and `survivors`
    /// (pid, start time) names the members still alive, where known.
    Stalled {
        evidence: String,
        survivors: Vec<(u32, u64)>,
    },
}

impl Teardown {
    pub(super) fn stalled(evidence: impl Into<String>) -> Self {
        Self::Stalled {
            evidence: evidence.into(),
            survivors: Vec::new(),
        }
    }

    /// This outcome, with `evidence` added as a further cause of a stall.
    pub(super) fn and_stalled(self, evidence: impl Into<String>) -> Self {
        let evidence = evidence.into();
        match self {
            Self::Confirmed => Self::stalled(evidence),
            Self::Stalled {
                evidence: earlier,
                survivors,
            } => Self::Stalled {
                evidence: format!("{earlier}; {evidence}"),
                survivors,
            },
        }
    }
}

/// Everything that confines one spawned host command.
pub(super) struct Tree {
    leader: Option<u32>,
    #[cfg(unix)]
    tracker: std::sync::Arc<std::sync::Mutex<archon_shell::process_tree::Tracker>>,
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
pub(super) use windows::{confine, kill_blocking, terminate_and_reap, terminate_completed_group};

/// Confines a child spawned as a session leader (`pre_exec` in the
/// supervisor): its tree is its group, its session, and its descendants.
#[cfg(unix)]
pub(super) fn confine(child: &mut tokio::process::Child) -> WorkflowResult<Tree> {
    let leader = child.id();
    let scope = archon_shell::process_tree::Scope {
        roots: leader.into_iter().collect(),
        groups: leader.into_iter().collect(),
        sessions: leader.into_iter().collect(),
    };
    Ok(Tree {
        leader,
        tracker: std::sync::Arc::new(std::sync::Mutex::new(
            archon_shell::process_tree::Tracker::new(scope),
        )),
    })
}

#[cfg(unix)]
impl Tree {
    /// Scan once while the command runs, remembering every member alive
    /// now. A failed scan loses only this scan.
    pub(super) async fn refresh(&self) {
        let tracker = self.tracker.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(mut tracker) = tracker.lock() {
                let _ = tracker.refresh();
            }
        })
        .await;
    }

    /// The leader is reaped: its pid may be reused, so ancestry from it ends.
    pub(super) fn leader_reaped(&self) {
        if let Ok(mut tracker) = self.tracker.lock() {
            tracker.scope_mut().roots.clear();
        }
    }

    async fn run(
        &self,
        work: impl FnOnce(&mut archon_shell::process_tree::Tracker) -> Teardown + Send + 'static,
    ) -> Teardown {
        let tracker = self.tracker.clone();
        tokio::task::spawn_blocking(move || match tracker.lock() {
            Ok(mut tracker) => work(&mut tracker),
            Err(_) => Teardown::stalled("host command tree tracker is poisoned"),
        })
        .await
        .unwrap_or_else(|error| Teardown::stalled(format!("teardown task failed: {error}")))
    }
}

/// Kill every member of the tree within `bound`.
#[cfg(unix)]
fn kill_tracked(
    tracker: &mut archon_shell::process_tree::Tracker,
    bound: std::time::Duration,
) -> Teardown {
    match tracker.kill(bound) {
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
            survivors: survivors.iter().map(|p| (p.pid, p.start)).collect(),
        },
        Err(error) => Teardown::stalled(format!("host command process tree scan failed: {error}")),
    }
}

/// Asks the tree to stop, kills what is left after the grace (every member
/// the SIGTERM round found stays remembered), and reaps the direct child
/// within [`REAP_DEADLINE`].
#[cfg(unix)]
pub(super) async fn terminate_and_reap(child: &mut tokio::process::Child, tree: &Tree) -> Teardown {
    let _ = tree
        .run(|tracker| match tracker.signal(libc::SIGTERM) {
            Ok(_) => Teardown::Confirmed,
            Err(error) => Teardown::stalled(error.to_string()),
        })
        .await;
    tokio::time::sleep(CLEANUP_GRACE).await;
    let teardown = tree
        .run(|tracker| kill_tracked(tracker, REAP_DEADLINE))
        .await;
    match reap(child).await {
        Ok(()) => teardown,
        Err(evidence) => teardown.and_stalled(evidence),
    }
}

/// After the leader exited on its own: kill whatever it left behind,
/// including members that escaped its group and session while it ran.
#[cfg(unix)]
pub(super) async fn terminate_completed_group(tree: &Tree) -> Teardown {
    tree.run(|tracker| kill_tracked(tracker, REAP_DEADLINE))
        .await
}

/// The drop guard's teardown: synchronous, because a drop cannot await.
#[cfg(unix)]
pub(super) fn kill_blocking(tree: &Tree, leader_unreaped: bool) -> Teardown {
    match tree.tracker.lock() {
        Ok(mut tracker) => {
            if !leader_unreaped {
                tracker.scope_mut().roots.clear();
            }
            kill_tracked(&mut tracker, REAP_DEADLINE)
        }
        Err(_) => Teardown::stalled("host command tree tracker is poisoned"),
    }
}

/// Reap the direct child within [`REAP_DEADLINE`]; the error is the evidence
/// of a stall.
pub(super) async fn reap(child: &mut tokio::process::Child) -> Result<(), String> {
    match tokio::time::timeout(REAP_DEADLINE, child.wait()).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => Err(format!("host command process reap failed: {error}")),
        Err(_) => Err("host command process reap exceeded cleanup deadline".to_string()),
    }
}
