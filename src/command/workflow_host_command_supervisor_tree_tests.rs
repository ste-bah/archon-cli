//! Issue 270: teardown reaches every process the host command started, not
//! only the members of its first process group.
//!
//! The descendants here leave the command's group the two ways real ones do:
//! a nested runner that gives each check its own group (`setpgrp`), and a
//! process that calls `setsid`. Perl is used because macOS has no `setsid`
//! command and `sh` has no portable way to change its group.
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::super::workflow_host_command_groups::{GROUP_RECORDS_DIR, left_groups, stalled_running};
use super::super::workflow_host_command_supervisor::{
    HostCommandControl, HostCommandSignal, supervise_process_group,
};
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

/// Waits, within a bound, for `pid` to leave the process table, and kills it
/// if it does not.
async fn assert_gone(pid: i32, cause: &str) {
    let start = std::time::Instant::now();
    // SAFETY: signal 0 only probes the process this test started.
    while unsafe { libc::kill(pid, 0) } == 0 {
        if start.elapsed() > Duration::from_secs(3) {
            drop(Kill(pid));
            panic!("descendant {pid} outlived {cause}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn read_pid(path: &Path) -> i32 {
    std::fs::read_to_string(path)
        .expect("descendant never started")
        .trim()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn completion_kills_a_setsid_descendant_seen_while_the_command_ran() {
    // Round 2, rule 2: the descendant leaves the group and the session, and
    // the command exits normally. A scan while it ran saw it.
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("daemon-pid");
    let body = format!(
        "perl -MPOSIX -e 'POSIX::setsid(); open(F, \">\", $ARGV[0]); print F $$; close F; sleep 30' '{}' </dev/null >/dev/null 2>&1 &\nuntil [ -s '{}' ]; do sleep 0.01; done\nsleep 1.5\nexit 0",
        pid_file.display(),
        pid_file.display()
    );
    let request = command(script(temp.path(), "seen-daemon", &body));
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(request, control, None)
        .await
        .unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert_gone(read_pid(&pid_file), "completion").await;
}

#[tokio::test]
async fn an_escaped_descendant_holding_the_pipes_pauses_instead_of_failing() {
    // Round 2, rule 3: a double-forked `setsid` descendant escapes before any
    // scan and keeps the output pipes open, so teardown cannot complete. That
    // is a stall: an operational, resumable outcome, never a failure.
    pause_scans();
    let temp = tempfile::tempdir().unwrap();
    let (program, pid_file) = escaping_pipe_holder(temp.path(), "exit 0");
    let (control, _handle) = HostCommandControl::new();
    let result = supervise_process_group(command(program), control, None).await;
    let _kill = Kill(read_pid(&pid_file));
    let output = result.expect("a stalled teardown is not a failure");
    assert_eq!(
        output.exit_code,
        Some(super::super::workflow_host_command_operational::EXIT_INCOMPLETE_RESUMABLE),
        "{output:?}"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("teardown"),
        "the evidence names the stall: {output:?}"
    );
}

/// A command whose double-forked `setsid` descendant escapes and holds the
/// output pipes, then (with `rest`) keeps running itself. The descendant
/// writes its pid only once its forked parent is gone and it leads its own
/// session (Issue 334: the pid used to be written while that parent could
/// still be exiting, and a teardown scan found the descendant through it
/// and killed it). On Linux the leader, a subreaper, adopts it until the
/// leader itself exits, which is before any teardown. A test that needs the
/// escape to come before every scan also calls [`pause_scans`]; the
/// periodic scan could otherwise see it on the way.
fn escaping_pipe_holder(dir: &Path, rest: &str) -> (PathBuf, PathBuf) {
    let pid_file = dir.join("daemon-pid");
    let body = format!(
        "perl -MPOSIX -e 'my $parent = $$; fork and exit; POSIX::setsid(); select(undef, undef, undef, 0.01) while getppid() == $parent; open(F, \">\", $ARGV[0]); print F $$; close F; sleep 30' '{pid}' &\nuntil [ -s '{pid}' ]; do sleep 0.01; done\n{rest}",
        pid = pid_file.display()
    );
    (script(dir, "escaping", &body), pid_file)
}

/// No periodic scan runs in a supervisor this test thread drives, so a
/// descendant that escapes is seen by no scan by construction.
fn pause_scans() {
    super::super::workflow_host_command_supervisor::SCANS_PAUSED.with(|paused| paused.set(true));
}

#[tokio::test]
async fn a_pause_returns_the_pause_even_when_teardown_stalls() {
    // Round 2, rule 3: a pause or cancel always comes back as itself; the
    // stall is evidence, not a replacement.
    let temp = tempfile::tempdir().unwrap();
    let (program, pid_file) = escaping_pipe_holder(temp.path(), "sleep 30");
    let (control, handle) = HostCommandControl::new();
    let task = tokio::spawn(supervise_process_group(command(program), control, None));
    while !pid_file.exists() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    handle
        .signal(super::super::workflow_host_command_supervisor::HostCommandSignal::Paused)
        .unwrap();
    let result = task.await.unwrap();
    let _kill = Kill(read_pid(&pid_file));
    assert!(
        matches!(
            result,
            Err(archon_workflow::WorkflowError::ControlPaused(_))
        ),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_stalled_teardown_keeps_the_resume_record() {
    pause_scans();
    let temp = tempfile::tempdir().unwrap();
    let records = temp.path().join("records");
    let (program, pid_file) = escaping_pipe_holder(temp.path(), "exit 0");
    let (control, _handle) = HostCommandControl::new();
    let result = supervise_process_group(command(program), control, Some(&records)).await;
    let _kill = Kill(read_pid(&pid_file));
    assert!(result.is_ok(), "{result:?}");
    let kept: Vec<_> = std::fs::read_dir(&records).unwrap().flatten().collect();
    assert_eq!(kept.len(), 1, "the record of a stalled teardown is kept");
    // The pipe holder escaped every scan: its survivors are unknown, and
    // that is what the record says.
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(kept[0].path()).unwrap()).unwrap();
    assert_eq!(record["stalled"], true, "{record}");
    assert_eq!(record["survivors_unknown"], true, "{record}");
}

#[tokio::test]
async fn a_confirmed_teardown_removes_the_resume_record() {
    let temp = tempfile::tempdir().unwrap();
    let records = temp.path().join("records");
    let program = script(temp.path(), "plain", "exit 0");
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(command(program), control, Some(&records))
        .await
        .unwrap();
    let left: Vec<_> = std::fs::read_dir(&records)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    assert!(
        left.is_empty(),
        "records left {left:?}; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn abort_before_first_scan_kills_an_original_session_descendant() {
    // No periodic scan runs in this test (the supervisor is driven on this
    // thread), so the abort is before the first scan by construction.
    pause_scans();
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("unscanned-child");
    let body = format!(
        "perl -e 'open(F, \">\", $ARGV[0]); print F $$; close F; sleep 30' '{}' </dev/null >/dev/null 2>&1 &\nsleep 30",
        pid_file.display()
    );
    let records = temp.path().join("records");
    let request = command(script(temp.path(), "abort-unscanned", &body));
    let (control, _handle) = HostCommandControl::new();
    let task =
        tokio::spawn(
            async move { supervise_process_group(request, control, Some(&records)).await },
        );
    let start = std::time::Instant::now();
    while std::fs::metadata(&pid_file).map_or(true, |m| m.len() == 0) {
        assert!(start.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let pid = read_pid(&pid_file);
    let _kill = Kill(pid);
    task.abort();
    let _ = task.await;
    assert_gone(pid, "abort before first scan").await;
}

#[tokio::test]
async fn overlapping_commands_are_running_siblings_not_stalled_teardowns() {
    // Round 5: a script may await several host commands at once
    // (`Promise.all`), so commands of one run overlap. Each live command
    // holds a record; the retry path read every such record as a stalled
    // teardown and paused the run beside a sibling that was only running.
    let temp = tempfile::tempdir().unwrap();
    let run_dir = temp.path().join("run");
    let records = run_dir.join(GROUP_RECORDS_DIR);
    let mut tasks = Vec::new();
    let mut handles = Vec::new();
    for name in ["sibling-a", "sibling-b"] {
        let request = command(script(temp.path(), name, "sleep 30"));
        let (control, handle) = HostCommandControl::new();
        let records = records.clone();
        tasks.push(tokio::spawn(async move {
            supervise_process_group(request, control, Some(&records)).await
        }));
        handles.push(handle);
    }
    let start = std::time::Instant::now();
    let count = || {
        std::fs::read_dir(&records).map_or(0, |entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
                .count()
        })
    };
    while count() < 2 {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "records never written"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let stalled = stalled_running(&run_dir).unwrap();
    let (running, _) = left_groups(&run_dir).unwrap();
    for handle in &handles {
        handle.signal(HostCommandSignal::Cancelled).unwrap();
    }
    for task in tasks {
        let _ = task.await;
    }
    assert!(
        stalled.is_empty(),
        "a running sibling read as a stalled teardown: {stalled:?}"
    );
    assert_eq!(
        running.len(),
        2,
        "both live commands still count as running"
    );
}
