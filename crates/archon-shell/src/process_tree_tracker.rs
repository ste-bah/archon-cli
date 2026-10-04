//! The members of a supervised tree, remembered across scans.
//!
//! A scope reaches a descendant through its group, its session, or a living
//! parent. A descendant that leaves the group and the session is reachable
//! only while its parent lives, so a tracker scans while the child runs and
//! keeps every member it has seen, pinned by its start time. Teardown then
//! covers what the scope reaches now and every pinned member still alive,
//! with their descendants.

use std::collections::BTreeMap;
use std::io;
use std::time::{Duration, Instant};

use super::identity::{Pinned, deliver};
use super::{Process, Scope, snapshot};

/// Rounds of stop-and-rescan before a kill. Each round stops the members a
/// scan found, so only a member forked between a scan and its stop can be
/// new on the next one; a fork bomb outruns any bound, so this one is small.
const FREEZE_ROUNDS: usize = 8;
const KILL_POLL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Default)]
pub struct Tracker {
    scope: Scope,
    /// pid -> start time of every member seen and still alive at the last
    /// scan.
    seen: BTreeMap<u32, u64>,
}

impl Tracker {
    pub fn new(scope: Scope) -> Self {
        Self {
            scope,
            seen: BTreeMap::new(),
        }
    }

    /// The scope, to drop a root once it is reaped.
    pub fn scope_mut(&mut self) -> &mut Scope {
        &mut self.scope
    }

    /// Whether `pid` with `start` is a member this tracker has seen.
    pub fn has_seen(&self, pid: u32, start: u64) -> bool {
        self.seen.get(&pid) == Some(&start)
    }

    /// Scan once: the live members now. Each one is remembered, and a
    /// remembered process that is gone (or whose pid now names another
    /// process) is forgotten.
    pub fn refresh(&mut self) -> io::Result<Vec<Process>> {
        let table = snapshot()?;
        let members = self.scope.reach(&table, &self.seen);
        let alive: BTreeMap<u32, u64> = table.iter().map(|p| (p.pid, p.start)).collect();
        self.seen.retain(|pid, start| alive.get(pid) == Some(start));
        for member in &members {
            self.seen.insert(member.pid, member.start);
        }
        Ok(members)
    }

    /// Send `signal` to every live member once; returns the members found.
    pub fn signal(&mut self, signal: i32) -> io::Result<Vec<u32>> {
        let members = self.refresh()?;
        for member in &members {
            deliver(member.pinned(), signal);
        }
        Ok(members.iter().map(|p| p.pid).collect())
    }

    /// Kill every member until none is left or `bound` runs out, and return
    /// the members still alive then (empty: the tree is confirmed empty).
    ///
    /// Members are stopped before they are killed, and the tree is scanned
    /// again after each stop: a member that forked between a scan and the
    /// kill would otherwise leave a child no later scan can tie to the tree
    /// once its parent is gone. The bound is checked before every scan and
    /// between freeze rounds.
    pub fn kill(&mut self, bound: Duration) -> io::Result<Vec<Pinned>> {
        let deadline = Instant::now() + bound;
        loop {
            let mut members = self.refresh()?;
            if members.is_empty() || Instant::now() >= deadline {
                return Ok(members.iter().map(Process::pinned).collect());
            }
            let mut stopped: BTreeMap<u32, u64> = BTreeMap::new();
            for _ in 0..FREEZE_ROUNDS {
                let fresh: Vec<Pinned> = members
                    .iter()
                    .filter(|p| stopped.get(&p.pid) != Some(&p.start))
                    .map(Process::pinned)
                    .collect();
                if fresh.is_empty() {
                    break;
                }
                for pinned in fresh {
                    deliver(pinned, libc::SIGSTOP);
                    stopped.insert(pinned.pid, pinned.start);
                }
                if Instant::now() >= deadline {
                    break;
                }
                members = self.refresh()?;
            }
            for (pid, start) in stopped {
                deliver(Pinned { pid, start }, libc::SIGKILL);
            }
            std::thread::sleep(KILL_POLL);
        }
    }
}
