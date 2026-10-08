//! Exercise the actual fallback sweep in an isolated process with a lowered limit.
use super::*;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;

fn regression(kind: &str) {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "process_tree::descriptors::bound_tests::lowered_limit_child",
            "--ignored",
        ])
        .env("ARCHON_FD_BOUND_CASE", kind)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("running 1 test"),
        "child filter did not run exactly one fixture"
    );
    assert!(
        output.status.success(),
        "{kind}: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn inherited_pipe_above_soft_limit_is_swept() {
    regression("pipe");
}
#[test]
fn inherited_socket_above_soft_limit_is_swept() {
    regression("socket");
}
#[test]
fn inherited_file_above_soft_limit_is_swept() {
    regression("file");
}

#[test]
#[ignore = "isolated resource limits"]
fn lowered_limit_child() {
    let Ok(kind) = std::env::var("ARCHON_FD_BOUND_CASE") else {
        return;
    };
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
        0
    );
    limit.rlim_cur = 8192;
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) }, 0);
    let (source, _peer): (OwnedFd, Option<OwnedFd>) = match kind.as_str() {
        "pipe" | "socket" => {
            let mut fds = [0; 2];
            let result = unsafe {
                if kind == "pipe" {
                    libc::pipe(fds.as_mut_ptr())
                } else {
                    libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr())
                }
            };
            assert_eq!(result, 0);
            unsafe {
                (
                    OwnedFd::from_raw_fd(fds[0]),
                    Some(OwnedFd::from_raw_fd(fds[1])),
                )
            }
        }
        "file" => (std::fs::File::open("/dev/null").unwrap().into(), None),
        _ => panic!("unknown descriptor fixture"),
    };
    let high = unsafe { libc::fcntl(source.as_raw_fd(), libc::F_DUPFD, 6000) };
    assert!(high >= 6000);
    let high = unsafe { OwnedFd::from_raw_fd(high) };
    limit.rlim_cur = 4096;
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) }, 0);
    let ceiling = descriptor_ceiling().unwrap();
    // A descriptor created after capture must also be covered: enumeration is
    // an upper bound, never a snapshot of the exact set to sweep.
    let late = unsafe { libc::fcntl(source.as_raw_fd(), libc::F_DUPFD, 4000) };
    assert!(late >= 4000);
    let late = unsafe { OwnedFd::from_raw_fd(late) };
    let script = format!(
        "test ! -e /dev/fd/{} && test ! -e /dev/fd/{} && echo ok",
        high.as_raw_fd(),
        late.as_raw_fd()
    );
    let mut child = std::process::Command::new("/bin/sh");
    child.args(["-c", &script]);
    unsafe {
        child.pre_exec(move || inherit_only_stdio_fallback(ceiling));
    }
    let output = child.output().unwrap();
    assert!(
        output.status.success(),
        "fallback retained fd above soft limit (ceiling {ceiling})"
    );
    assert_eq!(output.stdout, b"ok\n");
}
