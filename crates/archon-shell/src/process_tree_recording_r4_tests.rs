//! Checkpoint failures are evidence failures, not authority to abandon signals.
use super::*;
use std::io;
#[derive(Debug)]
struct FailingRecorder(bool);
impl IdentityRecorder for FailingRecorder {
    fn checkpoint(&mut self, _: &[Pinned]) -> io::Result<()> {
        if std::mem::replace(&mut self.0, false) {
            Ok(())
        } else {
            Err(io::Error::other("fixture marker became read-only"))
        }
    }
}
fn fixture() -> (tempfile::TempDir, std::process::Child, Tracker, Pinned) {
    let temp = tempfile::tempdir().unwrap();
    let child = group_leader("sleep 30 & echo $! > ready; wait", temp.path());
    wait_for(&temp.path().join("ready"));
    let descendant = std::fs::read_to_string(temp.path().join("ready"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let descendant = Pinned {
        pid: descendant,
        start: start_of(descendant).unwrap(),
    };
    let root = pin(&child);
    let mut tracker = Tracker::new(root, vec![root.pid], Vec::new());
    tracker
        .set_recorder(Box::new(FailingRecorder(true)))
        .unwrap();
    (temp, child, tracker, descendant)
}
fn cleanup(child: &mut std::process::Child, descendant: Pinned) {
    deliver(descendant, libc::SIGKILL);
    let _ = child.kill();
    let _ = child.wait();
}
#[test]
fn recording_failure_does_not_abort_adoption() {
    let (_temp, mut child, mut tracker, descendant) = fixture();
    let scan = tracker.refresh(Instant::now() + Duration::from_secs(2));
    cleanup(&mut child, descendant);
    assert!(
        scan.is_ok(),
        "checkpoint failure aborted identity adoption: {scan:?}"
    );
    assert!(tracker.has_seen(descendant.pid, descendant.start));
}
#[test]
fn recording_failure_does_not_abort_term() {
    let (_temp, mut child, mut tracker, descendant) = fixture();
    let signalled = tracker.signal(libc::SIGTERM, Instant::now() + Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(150));
    let exited = identity_of(descendant.pid).unwrap().is_none();
    cleanup(&mut child, descendant);
    assert!(
        signalled.is_ok(),
        "checkpoint failure prevented TERM: {signalled:?}"
    );
    assert!(exited, "TERM never reached descendant");
}
#[test]
fn recording_failure_does_not_abort_kill() {
    let (_temp, mut child, mut tracker, descendant) = fixture();
    let killed = tracker.kill(Duration::from_secs(2));
    let exited = identity_of(descendant.pid).unwrap().is_none();
    cleanup(&mut child, descendant);
    assert!(
        killed.is_ok(),
        "checkpoint failure prevented KILL: {killed:?}"
    );
    assert!(exited, "KILL never reached descendant");
}
