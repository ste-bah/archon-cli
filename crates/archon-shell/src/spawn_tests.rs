use std::io::Write;
use std::process::{Command, Stdio};

use super::{command, stdio_only, tokio_command};
use crate::process_tree::descriptors::bounded_ceiling;

/// A sibling's pipe caught in the window between std's `pipe()` and its
/// `FD_CLOEXEC`: both ends inheritable, moved above a shell's own
/// descriptors (bash keeps 255) where the limit allows. Closed on drop.
struct SiblingPipe {
    read: libc::c_int,
    write: libc::c_int,
}

impl SiblingPipe {
    fn new() -> Self {
        let mut ends = [0; 2];
        // SAFETY: pipe writes two descriptors into the array it is given.
        assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
        let floor = (crate::process_tree::descriptor_ceiling().unwrap() - 16).clamp(3, 300);
        let high = |fd: libc::c_int| {
            // SAFETY: F_DUPFD returns a new descriptor without FD_CLOEXEC;
            // close releases only the low copy this fixture made.
            let moved = unsafe { libc::fcntl(fd, libc::F_DUPFD, floor) };
            assert!(moved >= floor, "{}", std::io::Error::last_os_error());
            unsafe { libc::close(fd) };
            moved
        };
        let pipe = Self {
            read: high(ends[0]),
            write: high(ends[1]),
        };
        for fd in [pipe.read, pipe.write] {
            // SAFETY: reads only this descriptor's flags.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            assert_eq!(
                flags & libc::FD_CLOEXEC,
                0,
                "the fixture must be inheritable"
            );
        }
        pipe
    }

    /// A shell script that exits 0 only if neither end reached it.
    fn probe(&self) -> String {
        format!(
            "[ -e /dev/fd/{} ] && exit 3; [ -e /dev/fd/{} ] && exit 4; exit 0",
            self.read, self.write
        )
    }
}

impl Drop for SiblingPipe {
    fn drop(&mut self) {
        // SAFETY: closes only the descriptors this fixture opened.
        unsafe {
            libc::close(self.read);
            libc::close(self.write);
        }
    }
}

fn quiet(command: &mut Command) -> &mut Command {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
}

#[cfg(target_vendor = "apple")]
#[test]
fn a_sibling_pipe_does_not_reach_a_std_child() {
    let pipe = SiblingPipe::new();
    // Control: a plain std spawn (posix_spawn on macOS) does inherit it, so
    // the probe's "absent" below is a real absence.
    let plain = quiet(Command::new("/bin/sh").args(["-c", &pipe.probe()]))
        .status()
        .unwrap();
    assert_ne!(plain.code(), Some(0), "control: the pipe was not inherited");
    let status = quiet(command("/bin/sh").args(["-c", &pipe.probe()]))
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(0), "the sibling pipe reached the child");
}

#[cfg(target_vendor = "apple")]
#[tokio::test]
async fn a_sibling_pipe_does_not_reach_a_tokio_child() {
    let pipe = SiblingPipe::new();
    let status = tokio_command("/bin/sh")
        .args(["-c", &pipe.probe()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .unwrap();
    assert_eq!(status.code(), Some(0), "the sibling pipe reached the child");
}

#[cfg(target_vendor = "apple")]
#[test]
fn a_command_a_library_built_inherits_only_stdio_too() {
    let pipe = SiblingPipe::new();
    let mut built = Command::new("/bin/sh");
    built.args(["-c", &pipe.probe()]);
    let status = quiet(stdio_only(&mut built)).status().unwrap();
    assert_eq!(status.code(), Some(0), "the sibling pipe reached the child");
}

/// Off Apple targets the helper adds no hook, so std keeps `posix_spawn`
/// (no fork of a large parent): the race it guards against does not exist
/// there, as the next test shows for std's own pipes.
#[cfg(not(target_vendor = "apple"))]
#[test]
fn off_apple_the_helper_leaves_std_on_posix_spawn() {
    let pipe = SiblingPipe::new();
    let mut built = Command::new("/bin/sh");
    built.args(["-c", &pipe.probe()]);
    let status = quiet(stdio_only(&mut built)).status().unwrap();
    assert_ne!(status.code(), Some(0), "a hook swept the descriptors");
    let status = quiet(command("/bin/sh").args(["-c", &pipe.probe()]))
        .status()
        .unwrap();
    assert_ne!(status.code(), Some(0), "a hook swept the descriptors");
}

/// Linux std creates pipes close-on-exec atomically (`pipe2(O_CLOEXEC)`),
/// so no other thread's fork can catch one inheritable.
#[cfg(target_os = "linux")]
#[test]
fn linux_std_pipes_are_close_on_exec_from_creation() {
    use std::os::fd::AsRawFd;
    let (read, write) = std::io::pipe().unwrap();
    for fd in [read.as_raw_fd(), write.as_raw_fd()] {
        // SAFETY: reads only this descriptor's flags.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert_ne!(flags & libc::FD_CLOEXEC, 0, "fd {fd} is inheritable");
    }
}

#[tokio::test]
async fn stdio_still_works_for_std_and_tokio_children() {
    let script = "read line; echo \"out:$line\"; echo err >&2";
    let mut child = command("/bin/sh")
        .args(["-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"std\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        (output.stdout.as_slice(), output.stderr.as_slice()),
        (&b"out:std\n"[..], &b"err\n"[..])
    );

    let mut child = tokio_command("/bin/sh")
        .args(["-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use tokio::io::AsyncWriteExt;
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"tokio\n").await.unwrap();
    drop(stdin);
    let output = child.wait_with_output().await.unwrap();
    assert_eq!(
        (output.stdout.as_slice(), output.stderr.as_slice()),
        (&b"out:tokio\n"[..], &b"err\n"[..])
    );
}

#[tokio::test]
async fn a_program_that_cannot_start_is_still_an_error() {
    let missing = "/nonexistent/archon-issue-340-program";
    let error = command(missing).spawn().expect_err("std: cannot start");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "{error}");
    let error = tokio_command(missing)
        .spawn()
        .expect_err("tokio: cannot start");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "{error}");
}

#[test]
fn a_denied_sysctl_falls_back_to_the_soft_limit_bounded_by_the_table() {
    // The sysctl answered: the lower of the soft limit and the cap.
    assert_eq!(bounded_ceiling(1_048_576, Some(61_440), 61_440), 61_440);
    assert_eq!(bounded_ceiling(256, Some(61_440), 256), 256);
    // Denied: the soft limit still holds, bounded by the table size, so an
    // unlimited soft limit never becomes an unbounded sweep.
    assert_eq!(bounded_ceiling(libc::c_int::MAX, None, 10_240), 10_240);
    assert_eq!(bounded_ceiling(256, None, 10_240), 256);
}
