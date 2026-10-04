#![cfg_attr(target_os = "linux", allow(dead_code))]
//! Deadline-bounded probes with nonblocking stdout and one capped reaper.
//! A killed child stuck in kernel I/O retains one slot until try_wait proves
//! it exited. No timeout creates a reader or a blocking wait thread.
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::process::{Child, Command, Stdio};
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(10);
const CAPACITY: usize = 8;
static PROBES: Admission = Admission::new();
static STARTED: OnceLock<io::Result<()>> = OnceLock::new();

/// Probe slots and the killed probes still holding one. Production uses one
/// shared instance; a test can make its own and reap it by hand.
struct Admission {
    active: AtomicUsize,
    killed: Mutex<Vec<(Child, Slot)>>,
}

/// One admitted probe; dropping it frees the slot.
struct Slot(&'static AtomicUsize);
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Admission {
    const fn new() -> Self {
        Self {
            active: AtomicUsize::new(0),
            killed: Mutex::new(Vec::new()),
        }
    }

    fn reserve(&'static self) -> io::Result<Slot> {
        reserve(&self.active)?;
        Ok(Slot(&self.active))
    }

    /// One pass of the reaper: free the slot of every probe proven exited.
    fn reap(&self) {
        let mut children = self.killed.lock().unwrap_or_else(|e| e.into_inner());
        let mut index = 0;
        while index < children.len() {
            if matches!(children[index].0.try_wait(), Ok(Some(_))) {
                children.swap_remove(index);
            } else {
                index += 1;
            }
        }
    }

    /// Settle a probe. One already reaped (`exited`) frees its slot now;
    /// one not proven exited keeps it until a reap pass proves it.
    fn settle(&self, child: Child, slot: Slot, exited: bool) {
        if exited {
            drop((child, slot));
            return;
        }
        self.killed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((child, slot));
    }
}

fn reserve(count: &AtomicUsize) -> io::Result<()> {
    count
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
            (n < CAPACITY).then_some(n + 1)
        })
        .map(|_| ())
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "probe reaper is at capacity; survivors unknown",
            )
        })
}

fn start_reaper() -> io::Result<()> {
    super::cleanup::install_exit_drain()?;
    STARTED
        .get_or_init(|| {
            std::thread::Builder::new()
                .name("archon-probe-reaper".into())
                .spawn(|| {
                    loop {
                        PROBES.reap();
                        std::thread::sleep(POLL);
                    }
                })
                .map(|_| ())
        })
        .as_ref()
        .map(|_| ())
        .map_err(|error| io::Error::other(error.to_string()))
}

pub fn drain_probes(bound: Duration) -> bool {
    let deadline = Instant::now() + bound;
    while PROBES.active.load(Ordering::SeqCst) != 0 {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
    }
    true
}

/// All unsuccessful paths hand the child to the same nonblocking reaper.
/// Dropping stdout closes the pipe immediately, even if an escaped process
/// holds its write end: there is no reader thread left to wait for EOF.
pub(super) fn stdout_within(command: Command, deadline: Duration) -> io::Result<Vec<u8>> {
    start_reaper()?;
    probe_on(&PROBES, command, deadline)
}

fn probe_on(
    admission: &'static Admission,
    mut command: Command,
    deadline: Duration,
) -> io::Result<Vec<u8>> {
    let end = Instant::now() + deadline;
    let slot = admission.reserve()?;
    let mut exited = false;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn()?;
    let result = (|| {
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("probe stdout was not piped"))?;
        let fd = stdout.as_raw_fd();
        // SAFETY: fcntl receives this owned descriptor and plain flags.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut bytes = Vec::new();
        let mut buffer = [0; 8192];
        let mut eof = false;
        loop {
            if Instant::now() >= end {
                return Err(timed_out(deadline));
            }
            let read_more = match stdout.read(&mut buffer) {
                Ok(0) => {
                    eof = true;
                    false
                }
                Ok(count) => {
                    bytes.extend_from_slice(&buffer[..count]);
                    true
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    false
                }
                Err(error) => return Err(error),
            };
            exited = exited || child.try_wait()?.is_some();
            if exited && eof {
                return Ok(bytes);
            }
            // Read a continuously producing probe without sleeping each
            // buffer; the deadline is still checked on every iteration.
            if read_more {
                continue;
            }
            std::thread::sleep(POLL.min(end.saturating_duration_since(Instant::now())));
        }
    })();
    if result.is_err() && !exited {
        let _ = child.kill();
    }
    admission.settle(child, slot, exited);
    result
}

fn timed_out(deadline: Duration) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("probe did not finish within {deadline:?}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_admission_refuses_more_than_the_capacity() {
        let count = std::sync::atomic::AtomicUsize::new(0);
        for _ in 0..8 {
            assert!(reserve(&count).is_ok());
        }
        assert!(
            reserve(&count).is_err(),
            "a blocked reaper must bound admission"
        );
    }
    /// An admission of its own, with no reaper thread: only `reap` frees
    /// the slot of a killed probe, so nothing here depends on a tick.
    fn own_admission() -> &'static Admission {
        Box::leak(Box::new(Admission::new()))
    }

    #[test]
    fn a_finished_probe_frees_its_slot_at_once() {
        // Round 5: a probe that finished kept its slot until the reaper's next
        // tick, so more than the capacity in quick succession read as
        // "unknown survivors". Nothing reaps here: a slot kept is a failure.
        let admission = own_admission();
        for n in 0..CAPACITY * 2 {
            let mut command = Command::new("/bin/echo");
            command.arg("ok");
            let probed = probe_on(admission, command, Duration::from_secs(10));
            assert_eq!(
                probed
                    .as_ref()
                    .map(Vec::as_slice)
                    .map_err(|e| e.to_string()),
                Ok(&b"ok\n"[..]),
                "probe {n}"
            );
        }
        assert_eq!(admission.active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn repeated_timed_out_probes_have_bounded_resources() {
        // Each probe forks a setsid child that holds its stdout open. A killed
        // probe holds one slot until a reap proves it exited, never a thread,
        // and once reaped its slot is free again.
        let admission = own_admission();
        let dir = tempfile::tempdir().unwrap();
        let mut holders = Vec::new();
        let probe = |n: usize, holders: &mut Vec<i32>| {
            let path = dir.path().join(n.to_string());
            let mut command = Command::new("perl");
            command.args(["-MPOSIX", "-e", "if (fork() == 0) { POSIX::setsid(); open(F, '>', $ARGV[0]); print F $$; close F; sleep 30; exit 0 } sleep 30"]).arg(&path);
            let result = probe_on(admission, command, Duration::from_millis(150));
            if let Ok(pid) = std::fs::read_to_string(path)
                .and_then(|s| s.parse::<i32>().map_err(io::Error::other))
            {
                holders.push(pid);
            }
            result.map(|_| ()).map_err(|e| e.kind())
        };
        let mut outcomes = Vec::new();
        for n in 0..=CAPACITY {
            outcomes.push(probe(n, &mut holders));
        }
        // Proven exited is a fact the kernel reports, not a matter of time:
        // wait for it, within a bound that only guards a hung test.
        let start = Instant::now();
        while admission.active.load(Ordering::SeqCst) != 0
            && start.elapsed() < Duration::from_secs(10)
        {
            admission.reap();
            std::thread::sleep(POLL);
        }
        let freed = admission.active.load(Ordering::SeqCst);
        let after = probe(CAPACITY + 1, &mut holders);
        for pid in holders {
            // SAFETY: these pids belong to the processes this test spawned.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        let mut expected = vec![Err(io::ErrorKind::TimedOut); CAPACITY];
        expected.push(Err(io::ErrorKind::WouldBlock));
        assert_eq!(
            outcomes, expected,
            "killed probes hold their slots, up to the cap"
        );
        assert_eq!(freed, 0, "reaped probes still hold slots");
        assert_eq!(
            after,
            Err(io::ErrorKind::TimedOut),
            "a freed slot admits a probe"
        );
    }
}
