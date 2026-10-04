//! Windows-only: an owned job confines a child and its descendants, and its
//! emptiness is confirmed, not assumed.
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::time::{Duration, Instant};

use super::*;

/// A `cmd` that starts a detached background `ping` (the descendant that
/// outlives a direct-child kill) and then waits on a foreground one.
fn spawn_in(job: &Job) -> std::process::Child {
    let child = std::process::Command::new("cmd")
        .args([
            "/C",
            "start /B ping -n 30 127.0.0.1 >NUL & ping -n 30 127.0.0.1 >NUL",
        ])
        .creation_flags(CREATE_SUSPENDED_FLAG)
        .spawn()
        .unwrap();
    job.adopt_suspended(child.as_raw_handle(), child.id())
        .unwrap();
    child
}

fn wait_for_active(job: &Job, at_least: u32) {
    let start = Instant::now();
    while job.active_processes().unwrap() < at_least {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "descendants never started"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn kill_and_confirm_empties_a_job_whose_descendant_outlives_the_direct_child() {
    let job = Job::create(None).unwrap();
    let mut child = spawn_in(&job);
    // cmd plus two pings.
    wait_for_active(&job, 3);
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        job.active_processes().unwrap() > 0,
        "the descendants outlive the child"
    );
    assert_eq!(job.kill_and_confirm(Duration::from_secs(5)).unwrap(), 0);
    assert_eq!(job.active_processes().unwrap(), 0);
}

#[test]
fn a_named_job_runs_while_held_and_is_gone_once_dropped() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("Local\\archon-job-test-{}-{stamp}", std::process::id());
    let job = Job::create(Some(&name)).unwrap();
    assert_eq!(job.name(), Some(name.as_str()));
    let mut child = spawn_in(&job);
    wait_for_active(&job, 2);
    assert!(named_job_running(&name).unwrap());
    // A second owner of the same name is refused.
    assert_eq!(
        Job::create(Some(&name)).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    // ...and refusing it does not end the first owner's processes.
    assert!(job.active_processes().unwrap() >= 2);
    // A caller that needs the answer confirms first, on its own thread.
    assert_eq!(job.kill_and_confirm(Duration::from_secs(5)).unwrap(), 0);
    child.wait().unwrap();
    // Dropping the job returns at once (the close runs on its own thread)
    // and the job is gone soon after.
    let dropped = Instant::now();
    drop(job);
    assert!(
        dropped.elapsed() < Duration::from_millis(500),
        "drop blocked"
    );
    while named_job_running(&name).unwrap() {
        assert!(
            dropped.elapsed() < Duration::from_secs(10),
            "the job outlived its handle"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn an_unknown_job_name_is_not_running() {
    assert!(!named_job_running("Local\\archon-job-test-never-created").unwrap());
}
