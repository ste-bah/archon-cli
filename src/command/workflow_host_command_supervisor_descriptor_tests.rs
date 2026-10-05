//! Issue 334: a host command inherits its stdio and nothing else.
//!
//! Where std has no `pipe2` (macOS) a sibling command's pipe is inheritable
//! between its `pipe()` and its `FD_CLOEXEC`. A command forked in that
//! window used to carry the pipe into an escaped descendant, which held the
//! sibling's output open, so the sibling's clean teardown read as a stall
//! and its resume record was kept. The tests leave descriptors of this
//! process inheritable on purpose, which is that window held open.
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use super::super::workflow_host_command_supervisor::{HostCommandControl, supervise_process_group};
use super::{command, script};

/// An inheritable (not close-on-exec) copy of `source` at `at_least` or
/// above. Fixtures sit above the descriptors a shell keeps its script on
/// (10 for dash, 255 for bash) so a probe cannot mistake one for the other.
fn inheritable_copy(source: &impl AsRawFd, at_least: i32) -> OwnedFd {
    // SAFETY: F_DUPFD returns a new descriptor without FD_CLOEXEC.
    let raw = unsafe { libc::fcntl(source.as_raw_fd(), libc::F_DUPFD, at_least) };
    assert!(raw >= at_least, "{}", std::io::Error::last_os_error());
    // SAFETY: the descriptor was just created and is owned only here.
    let copy = unsafe { OwnedFd::from_raw_fd(raw) };
    // SAFETY: reads only this descriptor's flags.
    let flags = unsafe { libc::fcntl(raw, libc::F_GETFD) };
    assert_eq!(
        flags & libc::FD_CLOEXEC,
        0,
        "the fixture must be inheritable"
    );
    copy
}

/// The lowest fixture descriptor: above a shell's script descriptor where
/// the limit allows, and below the limit always.
fn fixture_floor() -> i32 {
    (archon_shell::process_tree::descriptor_ceiling().unwrap() - 16).clamp(3, 256)
}

/// A pipe whose two ends are inheritable, as std's are on macOS until it
/// sets `FD_CLOEXEC`.
fn inheritable_pipe() -> (OwnedFd, OwnedFd) {
    let mut fds = [0; 2];
    // SAFETY: pipe writes two new descriptors into the array it is given.
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    // SAFETY: both descriptors were just created and are owned only here.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let floor = fixture_floor();
    (
        inheritable_copy(&read, floor),
        inheritable_copy(&write, floor),
    )
}

/// A shell line that prints `name=held` or `name=free` for descriptor `fd`
/// of the shell itself.
fn probe(name: &str, fd: i32) -> String {
    format!("if [ -e /dev/fd/{fd} ]; then echo {name}=held; else echo {name}=free; fi")
}

#[tokio::test]
async fn a_sibling_pipe_left_inheritable_does_not_reach_the_command() {
    let temp = tempfile::tempdir().unwrap();
    let (read_end, write_end) = inheritable_pipe();
    let body = [
        probe("read", read_end.as_raw_fd()),
        probe("write", write_end.as_raw_fd()),
        // The probe sees a descriptor that is there: stdout itself.
        probe("stdout", 1),
    ]
    .join("\n");
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(
        command(script(temp.path(), "probe-pipe", &body)),
        control,
        None,
    )
    .await
    .unwrap();
    assert_eq!(output.exit_code, Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "read=free\nwrite=free\nstdout=held\n"
    );
}

#[tokio::test]
async fn a_high_numbered_inheritable_descriptor_does_not_reach_a_descendant() {
    // Far above stdio (and above the fixture floor), and probed from a
    // grandchild: nothing below the command can hold what the command
    // itself never had.
    let temp = tempfile::tempdir().unwrap();
    let at_least = (archon_shell::process_tree::descriptor_ceiling().unwrap() - 8).clamp(3, 4096);
    let null = std::fs::File::open("/dev/null").unwrap();
    let high = inheritable_copy(&null, at_least);
    let body = format!("/bin/sh -c '{}'", probe("high", high.as_raw_fd()));
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(
        command(script(temp.path(), "probe-high", &body)),
        control,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "high=free\n",
        "{output:?}"
    );
}

#[tokio::test]
async fn stdin_and_both_output_pipes_still_reach_the_command() {
    let temp = tempfile::tempdir().unwrap();
    let _leaked = inheritable_pipe();
    let mut request = command(script(
        temp.path(),
        "stdio",
        "read line\necho \"stdin=$line\"\necho err >&2",
    ));
    request.stdin = Some(b"ping\n".to_vec());
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(request, control, None)
        .await
        .unwrap();
    assert_eq!(output.exit_code, Some(0), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "stdin=ping\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "err\n");
}

#[tokio::test]
async fn a_program_that_cannot_start_is_still_an_io_error() {
    // The sweep flags descriptors and closes none, so std's own report of a
    // failed exec still arrives.
    let temp = tempfile::tempdir().unwrap();
    let mut request = command(script(temp.path(), "unused", "exit 0"));
    request.program = temp.path().join("no-such-program");
    let (control, _handle) = HostCommandControl::new();
    let result = supervise_process_group(request, control, None).await;
    assert!(
        matches!(result, Err(archon_workflow::WorkflowError::Io { .. })),
        "{result:?}"
    );
}
