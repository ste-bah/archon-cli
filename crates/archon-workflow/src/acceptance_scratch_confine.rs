//! What confines one check's processes, and its teardown (Issue 270).
//!
//! On Unix a check is a process-group leader. A [`Confinement`] tracks its
//! tree with an `archon_shell::process_tree::Tracker`: the leader is pinned,
//! its group id selects members while it is unreaped, and scans while the
//! check runs pin every descendant. The leader's exit is observed without
//! reaping it, the tree is torn down while the leader still holds its pid,
//! and only then is the leader reaped. A pid that now names another process
//! is never signalled. A stall
//! (a member that will not die, a probe that cannot finish, a leader that
//! cannot be reaped) is reported as the check's operational error, which is
//! resumable, never as a runner failure.
//!
//! On Windows the check's Job Object (`process_wrap`) is the confinement and
//! these calls do nothing.
use super::*;
use std::time::Duration;

/// How long teardown keeps killing before it reports survivors.
#[cfg(unix)]
const KILL_BOUND: Duration = Duration::from_secs(3);
/// How long a dropped runner's teardown thread keeps killing.
#[cfg(unix)]
const DROP_KILL_BOUND: Duration = Duration::from_secs(3);
/// How long one background scan may take; a scan past it is dropped whole.
#[cfg(unix)]
const SCAN_BUDGET: Duration = Duration::from_secs(2);
/// How often the leader's exit is polled (without reaping it).
#[cfg(unix)]
const EXIT_POLL: Duration = Duration::from_millis(20);
/// How long the leader's reap may take once it was killed.
const REAP_BOUND: Duration = Duration::from_secs(3);
#[cfg(unix)]
const HOLDER_PROBES: u32 = 10;

pub(super) struct Confinement {
    #[cfg(unix)]
    tracker: std::sync::Arc<std::sync::Mutex<archon_shell::process_tree::Tracker>>,
    /// Set once the runner stopped: a scan still running absorbs nothing.
    #[cfg(unix)]
    abandoned: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[cfg(unix)]
    scanning: Option<tokio::task::JoinHandle<()>>,
    armed: bool,
}

impl Confinement {
    #[cfg_attr(windows, allow(unused_variables))]
    pub(super) fn new(leader: u32) -> Self {
        Self {
            #[cfg(unix)]
            tracker: std::sync::Arc::new(std::sync::Mutex::new(
                archon_shell::process_tree::Tracker::new(
                    archon_shell::process_tree::Pinned {
                        pid: leader,
                        // An already exited leader is still our unreaped
                        // child; only its start time is unknown.
                        start: archon_shell::process_tree::start_of(leader).unwrap_or_default(),
                    },
                    vec![leader],
                    Vec::new(),
                ),
            )),
            #[cfg(unix)]
            abandoned: Default::default(),
            #[cfg(unix)]
            scanning: None,
            armed: true,
        }
    }

    /// Start one scan on a blocking thread unless one is still running, and
    /// return at once: the runner keeps watching its limits meanwhile. The
    /// table is read without the tracker lock, within its own budget; only
    /// a complete table is absorbed.
    pub(super) fn scan(&mut self) {
        #[cfg(unix)]
        if self
            .scanning
            .as_ref()
            .is_none_or(tokio::task::JoinHandle::is_finished)
        {
            let tracker = self.tracker.clone();
            let abandoned = self.abandoned.clone();
            self.scanning = Some(tokio::task::spawn_blocking(move || {
                let deadline = std::time::Instant::now() + SCAN_BUDGET;
                let Ok(table) = archon_shell::process_tree::snapshot_until(deadline) else {
                    return;
                };
                if abandoned.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                if let Ok(mut tracker) = tracker.lock() {
                    tracker.absorb(&table);
                }
            }));
        }
    }

    /// The leader is reaped: its pid may be reused, so its group id selects
    /// nothing any more.
    pub(super) fn leader_reaped(&self) {
        #[cfg(unix)]
        if let Ok(mut tracker) = self.tracker.lock() {
            tracker.leader_reaped();
        }
    }

    /// Kill the whole tree; `Err` is the evidence of a stall (survivors, or
    /// a tree that could not be scanned).
    pub(super) async fn kill(&self) -> Result<(), String> {
        #[cfg(unix)]
        {
            let tracker = self.tracker.clone();
            let killed = tokio::task::spawn_blocking(move || {
                let mut tracker = tracker.lock().map_err(|e| e.to_string())?;
                tracker.kill(KILL_BOUND).map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| format!("scratch teardown task failed: {e}"))?;
            match killed {
                Ok(survivors) if survivors.is_empty() => Ok(()),
                Ok(survivors) => Err(format!(
                    "scratch process tree teardown could not be verified (still alive: {})",
                    survivors
                        .iter()
                        .map(|p| p.pid.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                Err(error) => Err(format!("scratch teardown could not scan the tree: {error}")),
            }
        }
        #[cfg(not(unix))]
        Ok(())
    }

    /// Teardown confirmed the tree empty: nothing is left for the drop guard.
    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }

    /// The operational error for a process that still holds the scratch (or
    /// the build target) once the tree is gone, if any. Only a process this
    /// check is known to have started, or one holding something there for
    /// writing, is the check's: an unrelated reader (an editor, an indexer)
    /// is not. Polled for a short window, since a process the kill just
    /// reached releases its files as it exits. A probe that cannot finish
    /// proves nothing, which is an operational error too.
    #[cfg(unix)]
    pub(super) async fn detached_holders(
        &self,
        root: &Path,
        target: Option<&Path>,
    ) -> Option<String> {
        use archon_shell::process_tree::{HOLDER_PROBE_DEADLINE, Holder, holders_within};
        let roots: Vec<PathBuf> = std::iter::once(root)
            .chain(target)
            .map(Path::to_path_buf)
            .collect();
        let mut last: Vec<Holder> = Vec::new();
        for attempt in 0..HOLDER_PROBES {
            let probe_roots = roots.clone();
            let probe = tokio::task::spawn_blocking(move || {
                let roots: Vec<&Path> = probe_roots.iter().map(PathBuf::as_path).collect();
                holders_within(&roots, HOLDER_PROBE_DEADLINE)
            });
            // The probe bounds itself; this bound only guards that bound.
            let found = match tokio::time::timeout(HOLDER_PROBE_DEADLINE * 2, probe).await {
                Ok(Ok(Ok(found))) => found,
                Ok(Ok(Err(e))) => {
                    return Some(format!("scratch holder probe could not finish: {e}"));
                }
                Ok(Err(e)) => return Some(format!("scratch holder probe task failed: {e}")),
                Err(_) => return Some("scratch holder probe could not finish in time".into()),
            };
            // In a block of its own, so no lock guard lives across an await.
            last = {
                let tracker = self.tracker.lock().ok()?;
                found
                    .into_iter()
                    .filter(|h| {
                        h.writes || h.start.is_some_and(|start| tracker.has_seen(h.pid, start))
                    })
                    .collect()
            };
            if last.is_empty() {
                return None;
            }
            if attempt + 1 < HOLDER_PROBES {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        Some(format!(
            "processes the check left behind still use the scratch: {}",
            last.iter()
                .map(|h| format!("pid {} ({})", h.pid, h.path.display()))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    /// Windows confines a check to its Job Object; there is nothing outside
    /// it to find.
    #[cfg(not(unix))]
    pub(super) async fn detached_holders(
        &self,
        _root: &Path,
        _target: Option<&Path>,
    ) -> Option<String> {
        None
    }
}

impl Drop for Confinement {
    /// The runner stopped before its teardown. The teardown can take
    /// seconds, so it runs on a dedicated thread, never on the runtime; it
    /// treats the leader as reaped (the dropped child is about to be), and it
    /// only tries the tracker lock, within its bound.
    fn drop(&mut self) {
        // On Windows the child's Job Object (`process_wrap`'s `KillOnDrop`)
        // reaps the whole job when the child drops.
        #[cfg(unix)]
        if self.armed {
            self.abandoned
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let tracker = self.tracker.clone();
            let _ = std::thread::Builder::new()
                .name("archon-check-teardown".into())
                .spawn(move || {
                    let deadline = std::time::Instant::now() + DROP_KILL_BOUND;
                    while std::time::Instant::now() < deadline {
                        if let Ok(mut tracker) = tracker.try_lock() {
                            tracker.leader_reaped();
                            let _ = tracker.kill(DROP_KILL_BOUND);
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    tracing::warn!("scratch check teardown abandoned: its tree stayed busy");
                });
        }
    }
}

/// Resolves when the check's leader exits. On Unix it is not reaped: the
/// tree is torn down while the unreaped leader holds its pid.
pub(super) async fn leader_exit(child: &mut super::RunChild) -> std::io::Result<()> {
    #[cfg(unix)]
    {
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
    #[cfg(not(unix))]
    child.wait().await.map(drop)
}

/// Reap the leader within [`REAP_BOUND`]. A stall is written to `stall` and
/// the exit status is then unknown.
pub(super) async fn reap(
    child: &mut super::RunChild,
    stall: &mut Option<String>,
) -> Option<std::process::ExitStatus> {
    match tokio::time::timeout(REAP_BOUND, child.wait()).await {
        Ok(Ok(status)) => Some(status),
        Ok(Err(e)) => {
            *stall = Some(format!("scratch child could not be reaped: {e}"));
            None
        }
        Err(_) => {
            *stall = Some(format!(
                "scratch child was not reaped within {REAP_BOUND:?} after it was killed"
            ));
            None
        }
    }
}

/// Kill the check's whole tree while the leader is unreaped, then reap the
/// leader within [`REAP_BOUND`]. A stall is written to `stall` and the exit
/// status is then unknown.
pub(super) async fn terminate(
    child: &mut super::RunChild,
    confinement: &Confinement,
    stall: &mut Option<String>,
) -> Option<std::process::ExitStatus> {
    if let Err(evidence) = confinement.kill().await {
        *stall = Some(evidence);
    }
    // Windows: terminate the whole Job Object, not just the leader.
    #[cfg(windows)]
    let _ = child.start_kill();
    let status = reap(child, stall).await;
    confinement.leader_reaped();
    status
}
