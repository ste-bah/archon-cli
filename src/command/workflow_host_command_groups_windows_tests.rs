//! Resume checks recorded process identities before the named job scope.
use super::*;
use archon_shell::job_object::{CREATE_SUSPENDED_FLAG, Job};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::time::Duration;

fn crashed_record(run: &Path, name: Option<&str>) {
    let dir = run.join(GROUP_RECORDS_DIR);
    let guard = record_group(&dir, 42, 42, None, name, "cmd").unwrap();
    let path = guard.path().to_path_buf();
    std::mem::forget(guard);
    let mut record: HostCommandGroupRecord =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record.host_pid = u32::MAX;
    std::fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
}

fn name() -> String {
    format!("Local\\command-resume-test-{}", uuid::Uuid::new_v4())
}

#[test]
fn an_ambiguous_windows_marker_refuses_even_when_the_named_job_is_empty() {
    let run = tempfile::tempdir().unwrap();
    let name = name();
    let _job = Job::create(Some(&name)).unwrap();
    crashed_record(run.path(), Some(&name));
    // A legacy marker cannot name processes whose termination is still pending.
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let path = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "json"))
        .unwrap();
    let mut legacy: HostCommandGroupRecord =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    legacy.stalled = true;
    legacy.survivors_unknown = true;
    std::fs::write(
        path.with_extension("pending"),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();
    assert!(require_no_running_groups(run.path(), "run").is_err());
    assert_eq!(
        std::fs::read_dir(run.path().join(GROUP_RECORDS_DIR))
            .unwrap()
            .count(),
        2
    );
}

#[test]
fn a_crashed_windows_owners_marker_heals_when_the_named_job_is_gone() {
    let run = tempfile::tempdir().unwrap();
    let name = name();
    let job = Job::create(Some(&name)).unwrap();
    drop(job);
    crashed_record(run.path(), Some(&name));
    assert!(require_no_running_groups(run.path(), "run").is_ok());
}

#[test]
fn a_crashed_windows_owners_marker_waits_for_every_job_member() {
    let run = tempfile::tempdir().unwrap();
    let name = name();
    let job = Job::create(Some(&name)).unwrap();
    let mut child = std::process::Command::new("ping")
        .args(["-n", "30", "127.0.0.1"])
        .creation_flags(CREATE_SUSPENDED_FLAG)
        .spawn()
        .unwrap();
    job.adopt_suspended(child.as_raw_handle(), child.id())
        .unwrap();
    crashed_record(run.path(), Some(&name));
    let refused = require_no_running_groups(run.path(), "run");
    assert_eq!(job.kill_and_confirm(Duration::from_secs(5)).unwrap(), 0);
    child.wait().unwrap();
    assert!(refused.is_err(), "resume let a live job go");
    assert!(require_no_running_groups(run.path(), "run").is_ok());
}

#[test]
fn a_windows_record_without_job_evidence_stays_fail_closed() {
    let run = tempfile::tempdir().unwrap();
    crashed_record(run.path(), None);
    assert!(require_no_running_groups(run.path(), "run").is_err());
}

#[test]
fn a_windows_job_probe_error_stays_fail_closed() {
    let run = tempfile::tempdir().unwrap();
    // An empty name in a namespace is invalid, rather than a missing job.
    let invalid = r"Local\";
    assert!(archon_shell::job_object::named_job_running(invalid).is_err());
    crashed_record(run.path(), Some(invalid));
    assert!(require_no_running_groups(run.path(), "run").is_err());
}

#[test]
fn a_missing_job_name_does_not_hide_a_recorded_live_process() {
    let run = tempfile::tempdir().unwrap();
    let missing = name();
    let mut child = std::process::Command::new("ping")
        .args(["-n", "30", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    let start = archon_shell::job_object::identity_of(pid).unwrap().unwrap();
    let guard = record_group(
        &run.path().join(GROUP_RECORDS_DIR),
        pid,
        pid,
        None,
        Some(&missing),
        "cmd",
    )
    .unwrap();
    guard.keep(Some(&[(pid, start)]));
    let refused = require_no_running_groups(run.path(), "run");
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        refused.is_err(),
        "a disappearing job name allowed early healing"
    );
    assert!(require_no_running_groups(run.path(), "run").is_ok());
}

#[test]
fn unknown_survivors_outrank_an_empty_job() {
    let run = tempfile::tempdir().unwrap();
    let name = name();
    let _job = Job::create(Some(&name)).unwrap();
    record_group(
        &run.path().join(GROUP_RECORDS_DIR),
        42,
        42,
        None,
        Some(&name),
        "cmd",
    )
    .unwrap()
    .keep(None);
    assert!(require_no_running_groups(run.path(), "run").is_err());
}

#[test]
fn legacy_stall_without_identities_cannot_heal_on_a_missing_job() {
    let run = tempfile::tempdir().unwrap();
    record_group(
        &run.path().join(GROUP_RECORDS_DIR),
        42,
        42,
        None,
        Some(&name()),
        "cmd",
    )
    .unwrap()
    .keep(Some(&[]));
    assert!(require_no_running_groups(run.path(), "run").is_err());
}
