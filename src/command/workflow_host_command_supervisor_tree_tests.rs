//! Issue 270: teardown reaches every process the host command started, not
//! only the members of its first process group.
//!
//! The descendants here leave the command's group the two ways real ones do:
//! a nested runner that gives each check its own group (`setpgrp`), and a
//! process that calls `setsid`. Perl is used because macOS has no `setsid`
//! command and `sh` has no portable way to change its group.
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::super::workflow_host_command_groups::{GROUP_RECORDS_DIR, left_groups};
use super::super::workflow_host_command_supervisor::{HostCommandControl, supervise_process_group};
use super::{command, script};

/// A descendant that leaves the command's process group (`how` is the Perl
/// statement that does it), marks itself started, then writes `late` after
/// `delay` seconds unless it is killed first.
struct Escaper {
    ready: PathBuf,
    late: PathBuf,
}

impl Escaper {
    fn new(dir: &Path) -> Self {
        Self {
            ready: dir.join("escaper-ready"),
            late: dir.join("escaper-late"),
        }
    }

    fn start(&self, how: &str, delay: u32) -> String {
        format!(
            "perl -MPOSIX -e '{how}; open(F, \">\", $ARGV[0]); close F; sleep {delay}; open(F, \">\", $ARGV[1]); close F' '{ready}' '{late}' &\nuntil [ -e '{ready}' ]; do sleep 0.01; done",
            ready = self.ready.display(),
            late = self.late.display(),
        )
    }

    async fn assert_killed(&self, delay: u32, cause: &str) {
        assert!(self.ready.exists(), "descendant never started ({cause})");
        tokio::time::sleep(Duration::from_millis(u64::from(delay) * 1000 + 700)).await;
        assert!(!self.late.exists(), "descendant outlived {cause}");
    }
}

const OWN_GROUP: &str = "setpgid(0, 0)";
const OWN_SESSION: &str = "POSIX::setsid()";

#[tokio::test]
async fn timeout_kills_a_descendant_in_its_own_process_group() {
    // A freeze's probe check runs in its own group: the host command's group
    // kill never reached it, and the audit of that one group reported clean.
    let temp = tempfile::tempdir().unwrap();
    let escaper = Escaper::new(temp.path());
    let body = format!("{}\nsleep 30", escaper.start(OWN_GROUP, 2));
    let mut request = command(script(temp.path(), "timeout-group", &body));
    request.timeout_secs = 1;
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(request, control, None)
        .await
        .unwrap();
    assert!(output.timed_out);
    escaper.assert_killed(2, "the timeout").await;
}

#[tokio::test]
async fn completion_kills_a_descendant_left_in_its_own_process_group() {
    let temp = tempfile::tempdir().unwrap();
    let escaper = Escaper::new(temp.path());
    let body = format!("{}\nexit 0", escaper.start(OWN_GROUP, 2));
    let request = command(script(temp.path(), "complete-group", &body));
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(request, control, None)
        .await
        .unwrap();
    assert_eq!(output.exit_code, Some(0));
    escaper.assert_killed(2, "completion").await;
}

#[tokio::test]
async fn timeout_kills_a_setsid_descendant_whose_parent_still_runs() {
    // `setsid` leaves both the group and the session. While its parent is
    // alive the process is still the command's descendant, and teardown has
    // to find it by ancestry.
    let temp = tempfile::tempdir().unwrap();
    let escaper = Escaper::new(temp.path());
    let body = format!("{}\nsleep 30", escaper.start(OWN_SESSION, 2));
    let mut request = command(script(temp.path(), "timeout-session", &body));
    request.timeout_secs = 1;
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(request, control, None)
        .await
        .unwrap();
    assert!(output.timed_out);
    escaper.assert_killed(2, "the timeout").await;
}

/// Kills `pid` when the test ends, however it ends.
struct Kill(i32);

impl Drop for Kill {
    fn drop(&mut self) {
        // SAFETY: signals only the process this test started.
        unsafe {
            libc::kill(self.0, libc::SIGKILL);
        }
    }
}

#[test]
fn a_record_whose_group_ended_but_whose_session_still_runs_blocks_resume() {
    // The executor that wrote the record died. The command's own group is
    // gone, but a process it started in another group of its session (a
    // probe check) still runs and can still write the task root.
    let temp = tempfile::tempdir().unwrap();
    let member = temp.path().join("member");
    let mut leader = std::process::Command::new("perl")
        .args([
            "-MPOSIX",
            "-e",
            "POSIX::setsid(); my $pid = fork(); if ($pid == 0) { setpgid(0, 0); sleep 30; exit 0 } open(F, \">\", $ARGV[0]); print F $pid; close F; exit 0",
        ])
        .arg(&member)
        .spawn()
        .unwrap();
    let session = leader.id();
    assert!(leader.wait().unwrap().success());
    let member_pid: i32 = std::fs::read_to_string(&member)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let _kill = Kill(member_pid);

    let run_dir = temp.path().join("run");
    let records = run_dir.join(GROUP_RECORDS_DIR);
    std::fs::create_dir_all(&records).unwrap();
    std::fs::write(
        records.join(format!("{session}.json")),
        serde_json::json!({
            "schema_version": 1,
            "pgid": session,
            "pid": session,
            "session": session,
            "command_id": "test-host-command",
            "host_pid": 1,
            "started_at": "2026-10-04T00:00:00Z",
        })
        .to_string(),
    )
    .unwrap();
    let (running, ended) = left_groups(&run_dir).unwrap();
    assert_eq!(
        (running.len(), ended.len()),
        (1, 0),
        "a live member of the recorded session must keep the record running"
    );
}
