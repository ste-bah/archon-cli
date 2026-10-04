#![cfg_attr(target_os = "linux", allow(dead_code))]
//! A subprocess whose output is read within an end-to-end deadline.
//!
//! A probe such as `lsof` can block in the kernel on an unavailable
//! filesystem. Past the deadline the child is killed and the call returns
//! `TimedOut` at once: it neither waits for the kill to take effect nor for
//! the reader to see end of file, since a child stuck in the kernel may not
//! die until the operation it is blocked in ends.

use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(10);

/// The stdout of `command`, or `TimedOut` if it did not finish within
/// `deadline`. Its exit status is not judged here.
pub(super) fn stdout_within(mut command: Command, deadline: Duration) -> io::Result<Vec<u8>> {
    let end = Instant::now() + deadline;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("probe stdout was not piped"))?;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let read = stdout.read_to_end(&mut bytes).map(|_| bytes);
        let _ = sender.send(read);
    });
    loop {
        if child.try_wait()?.is_some() {
            let left = end.saturating_duration_since(Instant::now()).max(POLL);
            return receiver
                .recv_timeout(left)
                .map_err(|_| timed_out(deadline))?;
        }
        if Instant::now() >= end {
            let _ = child.kill();
            return Err(timed_out(deadline));
        }
        std::thread::sleep(POLL);
    }
}

fn timed_out(deadline: Duration) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("probe did not finish within {deadline:?}"),
    )
}
