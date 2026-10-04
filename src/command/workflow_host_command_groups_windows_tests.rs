//! Resume uses the existing named, non-breakaway Job Object as durable scope
//! evidence. No executor process needs to survive to answer this question.
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
fn a_crashed_windows_owners_marker_heals_when_the_named_job_is_empty() {
    let run = tempfile::tempdir().unwrap();
    let name = name();
    let _job = Job::create(Some(&name)).unwrap();
    crashed_record(run.path(), Some(&name));
    // The old marker format was indistinguishable from an unwritten
    // stall. Job containment still provides conclusive ended evidence.
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
    assert!(require_no_running_groups(run.path(), "run").is_ok());
    assert_eq!(
        std::fs::read_dir(run.path().join(GROUP_RECORDS_DIR))
            .unwrap()
            .count(),
        0
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
