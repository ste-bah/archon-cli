use std::os::unix::process::CommandExt;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use super::{Pinned, exited, identity_from_kinfo, identity_of, start_of};
use crate::process_tree::Activity;

/// A real group leader, blocked on stdin until the test closes the pipe.
struct Leader(Child);

impl Leader {
    fn spawn() -> Self {
        Self(
            crate::spawn::command("/bin/cat")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .unwrap(),
        )
    }

    fn pin(&self) -> Pinned {
        Pinned {
            pid: self.0.id(),
            start: start_of(self.0.id()).expect("live child start time"),
        }
    }

    fn exit_unreaped(&mut self) {
        drop(self.0.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(10);
        while !exited(self.0.id()).unwrap() {
            assert!(Instant::now() < deadline, "child never exited");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Leader {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn an_unreaped_child_and_then_a_reaped_child_have_no_live_identity() {
    let mut child = Leader::spawn();
    let pid = child.0.id();
    child.exit_unreaped();
    // kill(0) still succeeds, proving the child has not been reaped.
    // SAFETY: signal zero probes the real child's existence only.
    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, 0);
    assert_eq!(start_of(pid), None);
    assert_eq!(identity_from_kinfo(pid as libc::pid_t).unwrap(), None);
    assert_eq!(identity_of(pid).unwrap(), None);
    assert!(child.0.wait().unwrap().success());
    assert_eq!(identity_of(pid).unwrap(), None);
}

#[test]
fn a_live_child_has_the_same_microsecond_start_in_both_sources() {
    let child = Leader::spawn();
    let pin = child.pin();
    assert_eq!(identity_of(pin.pid).unwrap(), Some(pin.start));
    assert_eq!(
        identity_from_kinfo(pin.pid as libc::pid_t).unwrap(),
        Some(pin.start)
    );
}

#[test]
fn launchd_is_live_even_when_its_identity_is_permission_hidden() {
    let identity = identity_of(1);
    eprintln!("launchd identity: {identity:?}");
    assert!(
        matches!(identity, Ok(Some(start)) if start > 0),
        "launchd is live and sysctl reads it without privilege: {identity:?}"
    );
    // Exercise sysctl even on hosts where proc_pidinfo can read launchd.
    let fallback = identity_from_kinfo(1);
    eprintln!("launchd kinfo identity: {fallback:?}");
    assert_eq!(fallback.ok(), identity.ok());
}

#[test]
fn a_free_pid_has_no_identity() {
    // Find a real absent PID rather than relying on a fixed unused number.
    let pid = (2..100_000)
        .rev()
        .find(|pid| {
            // SAFETY: signal zero only probes existence.
            (unsafe { libc::kill(*pid as libc::pid_t, 0) }) < 0
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        })
        .expect("an unused pid");
    assert_eq!(identity_of(pid).unwrap(), None);
    assert_eq!(identity_from_kinfo(pid as libc::pid_t).unwrap(), None);
}

#[test]
fn activity_observes_a_pinned_zombie_leader_without_an_error() {
    let mut child = Leader::spawn();
    let pin = child.pin();
    let mut activity = Activity::default();
    let deadline = || Instant::now() + Duration::from_secs(5);
    assert!(!activity.observe(&[pin], deadline()).unwrap());
    child.exit_unreaped();
    assert!(activity.observe(&[pin], deadline()).unwrap());
    assert!(!activity.observe(&[pin], deadline()).unwrap());
}
