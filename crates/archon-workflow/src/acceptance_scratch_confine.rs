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

#[cfg(test)]
thread_local! {
    /// Test seam: while set on the thread that runs a check, its periodic
    /// scans do not run, so a test can act before any scan without racing
    /// the scan interval.
    pub(super) static SCANS_PAUSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) struct Confinement {
    #[cfg(unix)]
    tracker: std::sync::Arc<std::sync::Mutex<archon_shell::process_tree::Tracker>>,
    #[cfg(unix)]
    reaped: archon_shell::process_tree::ReapToken,
    /// Set once the runner stopped: a scan still running absorbs nothing.
    #[cfg(unix)]
    abandoned: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[cfg(unix)]
    scanning: Option<tokio::task::JoinHandle<()>>,
    #[cfg(windows)]
    job: Option<std::sync::Arc<archon_shell::job_object::Job>>,
    #[cfg(windows)]
    last_activity: Option<(i64, i64, u32, u32)>,
    activity_warned: std::sync::Arc<std::sync::atomic::AtomicBool>,
    armed: bool,
    progress: archon_shell::progress::Progress,
    #[cfg(unix)]
    activity: std::sync::Arc<std::sync::Mutex<archon_shell::process_tree::Activity>>,
}

impl Confinement {
    #[cfg_attr(windows, allow(unused_variables))]
    pub(super) fn new(leader: u32) -> Self {
        #[cfg(unix)]
        let tracker = archon_shell::process_tree::Tracker::new(
            archon_shell::process_tree::Pinned {
                pid: leader,
                start: archon_shell::process_tree::start_of(leader).unwrap_or_default(),
            },
            vec![leader],
            Vec::new(),
        );
        Self {
            #[cfg(unix)]
            reaped: tracker.reap_token(),
            #[cfg(unix)]
            tracker: std::sync::Arc::new(std::sync::Mutex::new(tracker)),
            #[cfg(unix)]
            abandoned: Default::default(),
            #[cfg(unix)]
            scanning: None,
            #[cfg(windows)]
            job: None,
            #[cfg(windows)]
            last_activity: None,
            activity_warned: Default::default(),
            armed: true,
            progress: archon_shell::progress::Progress::new(true),
            #[cfg(unix)]
            activity: Default::default(),
        }
    }

    #[cfg(windows)]
    pub(super) fn adopt(&mut self, child: &mut tokio::process::Child) -> WorkflowResult<()> {
        let job =
            archon_shell::job_object::Job::create(None).map_err(|e| invalid(e.to_string()))?;
        let (Some(handle), Some(pid)) = (child.raw_handle(), child.id()) else {
            return Err(invalid("check has no handle to confine"));
        };
        job.adopt_suspended(handle, pid)
            .map_err(|e| invalid(e.to_string()))?;
        self.job = Some(std::sync::Arc::new(job));
        Ok(())
    }

    pub(super) fn progress(&self) -> archon_shell::progress::Progress {
        self.progress.clone()
    }

    /// Start one scan on a blocking thread unless one is still running, and
    /// return at once: the runner keeps watching its limits meanwhile. The
    /// table is read without the tracker lock, within its own budget; only
    /// a complete table is absorbed.
    pub(super) fn scan(&mut self) {
        #[cfg(test)]
        if SCANS_PAUSED.with(std::cell::Cell::get) {
            return;
        }
        #[cfg(unix)]
        if self
            .scanning
            .as_ref()
            .is_none_or(tokio::task::JoinHandle::is_finished)
        {
            let tracker = self.tracker.clone();
            let abandoned = self.abandoned.clone();
            let activity = self.activity.clone();
            let progress = self.progress.clone();
            let warned = self.activity_warned.clone();
            self.scanning = Some(tokio::task::spawn_blocking(move || {
                let deadline = std::time::Instant::now() + SCAN_BUDGET;
                let observed = (|| -> std::io::Result<bool> {
                    let table = archon_shell::process_tree::snapshot_until(deadline)?;
                    if abandoned.load(std::sync::atomic::Ordering::SeqCst) {
                        return Ok(false);
                    }
                    let mut tracker = archon_shell::process_tree::lock_until(&tracker, deadline)?;
                    if abandoned.load(std::sync::atomic::Ordering::SeqCst) {
                        return Ok(false);
                    }
                    let pins = tracker.absorb_until(&table, deadline)?;
                    let mut activity = activity.lock().map_err(|_| {
                        std::io::Error::other("check activity sampler lock poisoned")
                    })?;
                    activity.observe(&pins, deadline)
                })();
                match observed {
                    Ok(true) => progress.record(),
                    Ok(false) => {}
                    Err(error) => {
                        if !warned.swap(true, std::sync::atomic::Ordering::SeqCst) {
                            tracing::warn!(%error, "check process activity is unreadable; no progress credited");
                        }
                    }
                }
            }));
        }
        #[cfg(windows)]
        if let Some(job) = &self.job {
            match job.activity_stamp() {
                Ok(stamp) => {
                    if self.last_activity.is_some_and(|last| last != stamp) {
                        self.progress.record();
                    }
                    self.last_activity = Some(stamp);
                }
                Err(error) => {
                    if !self
                        .activity_warned
                        .swap(true, std::sync::atomic::Ordering::SeqCst)
                    {
                        tracing::warn!(%error, "check job activity is unreadable; no progress credited");
                    }
                }
            }
        }
    }

    /// The leader is reaped: its pid may be reused, so its group id selects
    /// nothing any more.
    pub(super) fn leader_reaped(&self) {
        #[cfg(unix)]
        self.reaped.mark_reaped();
    }

    /// Kill the whole tree; `Err` is the evidence of a stall (survivors, or
    /// a tree that could not be scanned).
    pub(super) async fn kill(&self) -> Result<(), String> {
        #[cfg(unix)]
        {
            self.abandoned
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let tracker = self.tracker.clone();
            let deadline = std::time::Instant::now() + KILL_BOUND;
            let work = tokio::task::spawn_blocking(move || {
                let mut tracker = archon_shell::process_tree::lock_until(&tracker, deadline)
                    .map_err(|e| e.to_string())?;
                tracker
                    .kill(deadline.saturating_duration_since(std::time::Instant::now()))
                    .map_err(|e| e.to_string())
            });
            let killed = tokio::time::timeout(KILL_BOUND, work)
                .await
                .map_err(|_| {
                    "scratch teardown exceeded its deadline; survivors unknown".to_string()
                })?
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
        #[cfg(windows)]
        {
            let job = self
                .job
                .clone()
                .ok_or_else(|| "check job confinement missing".to_string())?;
            let killed =
                tokio::task::spawn_blocking(move || job.kill_and_confirm(Duration::from_secs(3)));
            match tokio::time::timeout(Duration::from_secs(4), killed).await {
                Ok(Ok(Ok(0))) => Ok(()),
                other => Err(format!("check job teardown unverified: {other:?}")),
            }
        }
        #[cfg(not(any(unix, windows)))]
        Err("check confinement unsupported on this platform".into())
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
                let Ok(tracker) = self.tracker.try_lock() else {
                    return Some("scratch holder identities are unknown: tracker busy".into());
                };
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

/// Own the unreaped child through cancellation as well as normal teardown.
/// Keeping its handle prevents PID/session reuse while the drop worker scans.
#[cfg(unix)]
pub(super) struct OwnedCheck {
    pub(super) child: std::mem::ManuallyDrop<super::RunChild>,
    pub(super) confinement: Confinement,
}
#[cfg(unix)]
impl OwnedCheck {
    pub(super) fn new(leader: u32, child: super::RunChild) -> Self {
        Self {
            child: std::mem::ManuallyDrop::new(child),
            confinement: Confinement::new(leader),
        }
    }
}
#[cfg(unix)]
impl Drop for OwnedCheck {
    fn drop(&mut self) {
        // SAFETY: this is the only take; the ManuallyDrop field is not dropped
        // again. The child stays owned by the worker until its final reap.
        let mut child = unsafe { std::mem::ManuallyDrop::take(&mut self.child) };
        if !self.confinement.armed {
            drop(child);
            return;
        }
        self.confinement
            .abandoned
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let tracker = self.confinement.tracker.clone();
        let reaped = self.confinement.reaped.clone();
        match std::thread::Builder::new()
            .name("archon-check-teardown".into())
            .spawn(move || {
                let deadline = std::time::Instant::now() + DROP_KILL_BOUND;
                let killed = archon_shell::process_tree::lock_until(&tracker, deadline).and_then(
                    |mut tracker| {
                        tracker.kill(deadline.saturating_duration_since(std::time::Instant::now()))
                    },
                );
                if !matches!(killed, Ok(ref survivors) if survivors.is_empty()) {
                    tracing::warn!(
                        ?killed,
                        "scratch drop teardown survivors are unknown or still alive"
                    );
                }
                // The leader remained unreaped during the scan/kill. Reap on a
                // private runtime so cleanup survives the caller's runtime exit.
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => {
                        let mut stall = None;
                        runtime.block_on(reap(&mut child, &mut stall));
                        if let Some(evidence) = stall {
                            tracing::warn!(%evidence, "scratch drop reap stalled");
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "scratch drop could not start reap runtime")
                    }
                }
                reaped.mark_reaped();
            }) {
            Ok(handle) => archon_shell::process_tree::register_cleanup(handle),
            Err(error) => {
                tracing::warn!(%error, "scratch check teardown thread could not start; survivors unknown")
            }
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
