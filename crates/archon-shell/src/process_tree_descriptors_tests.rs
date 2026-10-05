use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use super::{descriptor_ceiling, inherit_only_stdio};

/// An inheritable (not close-on-exec) descriptor: what a pipe looks like
/// between std's `pipe()` and its `FD_CLOEXEC` on a platform without
/// `pipe2`. It sits above a shell's script descriptor (bash keeps 255) where
/// the limit allows. Closed on drop.
struct Inheritable(libc::c_int);

impl Inheritable {
    fn new() -> Self {
        use std::os::fd::AsRawFd;
        let floor = (descriptor_ceiling().unwrap() - 16).clamp(3, 256);
        let null = std::fs::File::open("/dev/null").unwrap();
        // SAFETY: F_DUPFD returns a new descriptor without FD_CLOEXEC.
        let fd = unsafe { libc::fcntl(null.as_raw_fd(), libc::F_DUPFD, floor) };
        assert!(fd >= floor, "{}", std::io::Error::last_os_error());
        // SAFETY: reads only this descriptor's flags.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert_eq!(
            flags & libc::FD_CLOEXEC,
            0,
            "the fixture must be inheritable"
        );
        Self(fd)
    }
}

impl Drop for Inheritable {
    fn drop(&mut self) {
        // SAFETY: closes only the descriptor this fixture opened.
        unsafe {
            libc::close(self.0);
        }
    }
}

/// Whether a shell exec'd with (`hook`) or without the sweep still has `fd`.
fn shell_holds(fd: libc::c_int, hook: bool) -> bool {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", &format!("[ -e /dev/fd/{fd} ]")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if hook {
        let ceiling = descriptor_ceiling().unwrap();
        // SAFETY: the hook calls fcntl/close_range only.
        unsafe {
            command.pre_exec(move || inherit_only_stdio(ceiling));
        }
    }
    command.status().unwrap().success()
}

#[test]
fn an_inheritable_descriptor_does_not_reach_the_exec_d_program() {
    let leaked = Inheritable::new();
    // The probe sees an inherited descriptor without the sweep, so its
    // "absent" below is a real absence.
    assert!(
        shell_holds(leaked.0, false),
        "control: fd {} not inherited",
        leaked.0
    );
    assert!(
        !shell_holds(leaked.0, true),
        "fd {} reached the child",
        leaked.0
    );
}

#[test]
fn stdio_survives_the_sweep() {
    let ceiling = descriptor_ceiling().unwrap();
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "read line; echo \"out:$line\"; echo err >&2"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: the hook calls fcntl/close_range only.
    unsafe {
        command.pre_exec(move || inherit_only_stdio(ceiling));
    }
    let mut child = command.spawn().unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(b"ping\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"out:ping\n");
    assert_eq!(output.stderr, b"err\n");
}

#[test]
fn an_exec_failure_is_still_reported() {
    // std reports a failed exec through a close-on-exec pipe of its own; the
    // sweep flags descriptors and never closes one, so that report arrives.
    let ceiling = descriptor_ceiling().unwrap();
    let mut command = Command::new("/nonexistent/archon-issue-334-program");
    // SAFETY: the hook calls fcntl/close_range only.
    unsafe {
        command.pre_exec(move || inherit_only_stdio(ceiling));
    }
    let error = command.spawn().expect_err("a missing program cannot start");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "{error}");
}
