//! Issue-134: a Bash call's process tree dies with the call, however the
//! call ends.
//!
//! Every agent Bash command runs as the leader of its own session and
//! process group (`bash_output::spawn_wrapped_child`). A call that finishes
//! tears the group down itself (`terminate_completed_process_tree`,
//! `terminate_child`). A call whose FUTURE is dropped -- a run pause or
//! cancel (`archon_workflow::control_race` drops the in-flight work), a
//! branch timeout, a panic, an executor unwinding -- never reached that code:
//! the only thing left was tokio's kill-on-drop, which SIGKILLs the group
//! LEADER alone. Its descendants were re-parented to launchd and ran on;
//! live, a paused unit's `sh /tmp/<check>.sh -> cargo run` ran for 1h42m and
//! ignored SIGTERM.
//!
//! [`LiveGroup`] is armed when the command is spawned and ends the tree when
//! dropped: it snapshots the group's members and every descendant of theirs
//! (by parent pid, so a descendant that left the group with `setsid` is
//! found while its parent chain is intact -- one whose parent already died
//! and was re-parented to init in a new session is beyond any sweep on
//! macOS), sends SIGTERM, waits [`GRACE`], then SIGKILLs whatever is
//! left, before the drop returns. The call's record is written after the
//! future that owned it is gone, so nothing it started outlives it into
//! that record. A call that finished normally disarms it once its own
//! cleanup ran (the group id is free from then on and must not be signalled
//! blindly).
//!
//! Executor death (the host process itself killed) cannot run a destructor;
//! the in-shell watcher in `bash_containment` covers that case.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

/// How long a TERM'd tree gets before it is killed.
pub(super) const GRACE: Duration = Duration::from_millis(500);

/// Every armed group, by id: the session (inside a workflow run, the run
/// id) it belongs to, and whether it was already ended from outside.
static LIVE: Mutex<BTreeMap<u32, (String, bool)>> = Mutex::new(BTreeMap::new());

fn live() -> std::sync::MutexGuard<'static, BTreeMap<u32, (String, bool)>> {
    LIVE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Ends the process tree of group `pgid` when dropped, unless finished.
pub(crate) struct LiveGroup(Option<u32>);

impl LiveGroup {
    pub(crate) fn arm(pgid: Option<u32>, session: &str) -> Self {
        if let Some(pgid) = pgid {
            live().insert(pgid, (session.to_string(), false));
        }
        Self(pgid)
    }

    /// Stop guarding; `true` when it was still ours to end.
    fn release(&mut self) -> Option<u32> {
        let pgid = self.0.take()?;
        match live().remove(&pgid) {
            Some((_, true)) => None,
            _ => Some(pgid),
        }
    }

    /// The call's own cleanup ran: end whatever of the tree survived it,
    /// while the group still has members (its id cannot be reused while it
    /// does), then stop guarding.
    pub(crate) fn finish(mut self) {
        if let Some(pgid) = self.release()
            && group_exists(pgid)
        {
            end_trees(&[pgid], Duration::ZERO);
        }
    }
}

impl Drop for LiveGroup {
    fn drop(&mut self) {
        if let Some(pgid) = self.release() {
            let survivors = end_trees(&[pgid], GRACE);
            if !survivors.is_empty() {
                tracing::warn!(
                    process_group = pgid,
                    ?survivors,
                    "bash: process tree survived an abandoned call"
                );
            }
        }
    }
}

/// End, now, every armed Bash tree whose session contains `key` (inside a
/// workflow run: the run id), all under one shared grace, and return what
/// survived. For the host to call when it abandons a run's calls (a pause
/// or cancel), BEFORE it records them as ended: a call's own futures may
/// still be waiting to be dropped on another thread (a tool fanned out in a
/// task), and their guards then find their groups already ended.
pub fn end_process_groups_of(key: &str) -> Vec<u32> {
    let pgids: Vec<u32> = {
        let mut live = live();
        (live.iter_mut())
            .filter(|(_, (session, ended))| !*ended && session.contains(key))
            .map(|(pgid, (_, ended))| {
                *ended = true;
                *pgid
            })
            .collect()
    };
    if pgids.is_empty() {
        return Vec::new();
    }
    end_trees(&pgids, GRACE)
}

/// Aborts a task when dropped: a Bash call's heartbeat must not outlive it.
pub(crate) struct AbortOnDrop(pub(crate) Option<tokio::task::JoinHandle<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

/// Whether group `pgid` still has a member (a zombie leader included).
fn group_exists(pgid: u32) -> bool {
    #[cfg(unix)]
    {
        let alive = unsafe { libc::kill(-(pgid as libc::pid_t), 0) } == 0;
        alive || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
    #[cfg(not(unix))]
    {
        let _ = pgid;
        false
    }
}

/// `(pid, ppid, pgid, zombie)` of every process, from `ps`.
#[cfg(unix)]
fn processes() -> Vec<(u32, u32, u32, bool)> {
    let Ok(output) = archon_shell::spawn::command("ps")
        .args(["-axo", "pid=,ppid=,pgid=,stat="])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let ppid = fields.next()?.parse().ok()?;
            let pgid = fields.next()?.parse().ok()?;
            let zombie = fields.next().is_some_and(|stat| stat.starts_with('Z'));
            Some((pid, ppid, pgid, zombie))
        })
        .collect()
}

/// The live members of the groups `pgids` and all their live descendants.
#[cfg(unix)]
fn trees(pgids: &[u32]) -> Vec<u32> {
    let table = processes();
    let own = std::process::id();
    let mut found: Vec<u32> = (table.iter())
        .filter(|(pid, _, group, zombie)| {
            pgids.contains(group) && !zombie && *pid != own && *pid > 1
        })
        .map(|(pid, ..)| *pid)
        .collect();
    let mut at = 0;
    while at < found.len() {
        let parent = found[at];
        for (pid, ppid, _, zombie) in &table {
            if *ppid == parent && !zombie && *pid != own && *pid > 1 && !found.contains(pid) {
                found.push(*pid);
            }
        }
        at += 1;
    }
    found
}

#[cfg(unix)]
fn signal(pgids: &[u32], pids: &[u32], signal: libc::c_int) {
    unsafe {
        for pgid in pgids {
            libc::kill(-(*pgid as libc::pid_t), signal);
        }
        for pid in pids {
            libc::kill(*pid as libc::pid_t, signal);
        }
    }
}

/// TERM every process of the trees, give them `grace` (shared), KILL what is
/// left; returns what survived even that.
#[cfg(unix)]
pub(super) fn end_trees(pgids: &[u32], grace: Duration) -> Vec<u32> {
    use std::time::Instant;
    let pids = trees(pgids);
    if pids.is_empty() {
        return Vec::new();
    }
    let alive = |pids: &[u32]| -> Vec<u32> {
        let table = processes();
        let running = |pid: &u32| table.iter().any(|(p, _, _, zombie)| p == pid && !zombie);
        (pids.iter().copied())
            .filter(|pid| running(pid))
            .chain(trees(pgids))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    };
    if !grace.is_zero() {
        signal(pgids, &pids, libc::SIGTERM);
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline && !alive(&pids).is_empty() {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    let left = alive(&pids);
    if left.is_empty() {
        return left;
    }
    signal(pgids, &left, libc::SIGKILL);
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let left = alive(&left);
        if left.is_empty() || Instant::now() >= deadline {
            return left;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(not(unix))]
pub(super) fn end_trees(_pgids: &[u32], _grace: Duration) -> Vec<u32> {
    Vec::new()
}
