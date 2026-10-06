//! The pipe tests are Apple only: Linux std makes its pipes close-on-exec
//! atomically, so `archon_shell::spawn` adds no sweep there (Issue 340).
#[cfg(target_vendor = "apple")]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_vendor = "apple")]
use std::path::Path;

use super::daemon_command;

#[cfg(target_vendor = "apple")]
/// A sibling's pipe caught between std's `pipe()` and its `FD_CLOEXEC`:
/// both ends inheritable, moved above a shell's own descriptors. Closed on
/// drop.
struct SiblingPipe([libc::c_int; 2]);

#[cfg(target_vendor = "apple")]
impl SiblingPipe {
    fn new() -> Self {
        let mut ends = [0; 2];
        // SAFETY: pipe writes two descriptors into the array it is given.
        assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
        let floor = (archon_shell::process_tree::descriptor_ceiling().unwrap() - 16).clamp(3, 300);
        for end in &mut ends {
            // SAFETY: F_DUPFD returns a new descriptor without FD_CLOEXEC;
            // close releases only the low copy made above.
            let moved = unsafe { libc::fcntl(*end, libc::F_DUPFD, floor) };
            assert!(moved >= floor, "{}", std::io::Error::last_os_error());
            unsafe { libc::close(*end) };
            *end = moved;
        }
        Self(ends)
    }
}

#[cfg(target_vendor = "apple")]
impl Drop for SiblingPipe {
    fn drop(&mut self) {
        // SAFETY: closes only the descriptors this fixture opened.
        unsafe {
            libc::close(self.0[0]);
            libc::close(self.0[1]);
        }
    }
}

#[cfg(target_vendor = "apple")]
/// A stand-in daemon executable: it records its arguments and whether either
/// end of `pipe` reached it, then exits.
fn probe_daemon(dir: &Path, pipe: &SiblingPipe) -> std::path::PathBuf {
    let report = dir.join("report");
    let script = format!(
        "#!/bin/sh\nif [ -e /dev/fd/{} ] || [ -e /dev/fd/{} ]; then held=held; else held=absent; fi\n\
         printf '%s|%s' \"$held\" \"$*\" > '{}'\n",
        pipe.0[0],
        pipe.0[1],
        report.display()
    );
    let exe = dir.join("daemon");
    std::fs::write(&exe, script).unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    exe
}

#[cfg(target_vendor = "apple")]
fn report(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("report")).unwrap()
}

#[cfg(target_vendor = "apple")]
#[test]
fn a_sibling_pipe_does_not_reach_the_cognitive_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = SiblingPipe::new();
    let exe = probe_daemon(dir.path(), &pipe);
    // Control: a plain spawn of the same probe does inherit the pipe, so the
    // "absent" below is a real absence.
    let status = std::process::Command::new(&exe).status().unwrap();
    assert!(status.success());
    assert_eq!(report(dir.path()), "held|", "control: pipe not inherited");

    let status = daemon_command(&exe, dir.path(), Some(250))
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        report(dir.path()),
        "absent|cognitive daemon run --interval-ms 250",
        "the sibling pipe reached the daemon"
    );
}

#[test]
fn a_daemon_that_cannot_start_is_still_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-such-archon");
    let error = daemon_command(&missing, dir.path(), None)
        .spawn()
        .expect_err("a missing executable cannot start");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "{error}");
}
