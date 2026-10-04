//! What confines one check's processes, and its teardown (Issue 270).
//!
//! On Unix a check is a process-group leader. A [`Confinement`] tracks its
//! tree: the group, every descendant by ancestry while the leader is
//! unreaped, and every member a scan saw while the check ran, pinned by its
//! start time (`archon_shell::process_tree::Tracker`). Teardown kills all of
//! them; a pid that now names another process is never signalled. A stall
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
/// How long a dropped runner keeps killing; a drop cannot wait long.
#[cfg(unix)]
const DROP_KILL_BOUND: Duration = Duration::from_millis(500);
/// How long the leader's reap may take once it was killed.
const REAP_BOUND: Duration = Duration::from_secs(3);
#[cfg(unix)]
const HOLDER_PROBES: u32 = 10;

pub(super) struct Confinement {
    #[cfg(unix)]
    tracker: std::sync::Arc<std::sync::Mutex<archon_shell::process_tree::Tracker>>,
    armed: bool,
}

impl Confinement {
    #[cfg_attr(windows, allow(unused_variables))]
    pub(super) fn new(leader: u32) -> Self {
        Self {
            #[cfg(unix)]
            tracker: std::sync::Arc::new(std::sync::Mutex::new(
                archon_shell::process_tree::Tracker::new(archon_shell::process_tree::Scope {
                    roots: vec![leader],
                    groups: vec![leader],
                    sessions: Vec::new(),
                }),
            )),
            armed: true,
        }
    }

    /// Scan once, remembering every member now alive. A failed scan only
    /// loses this scan; teardown scans again.
    pub(super) async fn refresh(&self) {
        #[cfg(unix)]
        {
            let tracker = self.tracker.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if let Ok(mut tracker) = tracker.lock() {
                    let _ = tracker.refresh();
                }
            })
            .await;
        }
    }

    /// The leader is reaped: its pid may be reused, so ancestry from it ends.
    pub(super) fn leader_reaped(&self) {
        #[cfg(unix)]
        if let Ok(mut tracker) = self.tracker.lock() {
            tracker.scope_mut().roots.clear();
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
    fn drop(&mut self) {
        // On Windows the child's Job Object (`process_wrap`'s `KillOnDrop`)
        // reaps the whole job when the child drops.
        #[cfg(unix)]
        if self.armed
            && let Ok(mut tracker) = self.tracker.lock()
        {
            let _ = tracker.kill(DROP_KILL_BOUND);
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
