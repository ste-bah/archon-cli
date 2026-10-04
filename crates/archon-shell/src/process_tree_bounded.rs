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
static ACTIVE_PROBES: AtomicUsize = AtomicUsize::new(0);
static REAPER: Mutex<Vec<(Child, Slot)>> = Mutex::new(Vec::new());
static STARTED: OnceLock<io::Result<()>> = OnceLock::new();

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
struct Slot;
impl Drop for Slot {
    fn drop(&mut self) {
        ACTIVE_PROBES.fetch_sub(1, Ordering::SeqCst);
    }
}

fn start_reaper() -> io::Result<()> {
    super::cleanup::install_exit_drain()?;
    STARTED
        .get_or_init(|| {
            std::thread::Builder::new()
                .name("archon-probe-reaper".into())
                .spawn(|| {
                    loop {
                        {
                            let mut children = REAPER.lock().unwrap_or_else(|e| e.into_inner());
                            let mut index = 0;
                            while index < children.len() {
                                if matches!(children[index].0.try_wait(), Ok(Some(_))) {
                                    children.swap_remove(index);
                                } else {
                                    index += 1;
                                }
                            }
                        }
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
    while ACTIVE_PROBES.load(Ordering::SeqCst) != 0 {
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
pub(super) fn stdout_within(mut command: Command, deadline: Duration) -> io::Result<Vec<u8>> {
    let end = Instant::now() + deadline;
    start_reaper()?;
    reserve(&ACTIVE_PROBES)?;
    let slot = Slot;
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
            if child.try_wait()?.is_some() && eof {
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
    if result.is_err() {
        let _ = child.kill();
    }
    REAPER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((child, slot));
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
    #[test]
    fn repeated_timed_out_probes_have_bounded_resources() {
        let dir = tempfile::tempdir().unwrap();
        let mut holders = Vec::new();
        for n in 0..12 {
            let path = dir.path().join(n.to_string());
            let mut command = Command::new("perl");
            command.args(["-MPOSIX", "-e", "if (fork() == 0) { POSIX::setsid(); open(F, '>', $ARGV[0]); print F $$; close F; sleep 30; exit 0 } sleep 30"]).arg(&path);
            assert!(stdout_within(command, Duration::from_millis(150)).is_err());
            if let Ok(pid) = std::fs::read_to_string(path)
                .and_then(|s| s.parse::<i32>().map_err(io::Error::other))
            {
                holders.push(pid);
            }
        }
        let active = ACTIVE_PROBES.load(std::sync::atomic::Ordering::SeqCst);
        for pid in holders {
            // SAFETY: these pids belong to the processes this test spawned.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        assert!(
            active <= 8,
            "timed-out probes left {active} blocked readers"
        );
    }
}
