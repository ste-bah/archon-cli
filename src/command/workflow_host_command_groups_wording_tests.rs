//! Issue 522: every resume refusal names the run it refuses and no run kind,
//! on every host, because generic and fixed resumes share these texts.
use super::*;

fn record(dir: &Path, pid: u32, unknown: bool) -> HostCommandGroupRecord {
    HostCommandGroupRecord {
        schema_version: 1,
        pgid: 4242,
        pid,
        session: None,
        job: None,
        survivors: Vec::new(),
        stalled: unknown,
        survivors_unknown: unknown,
        teardown_complete: false,
        command_id: "CHECK-1".into(),
        host_pid: 777,
        host_start: None,
        leader_start: None,
        started_at: "2026-10-29T00:00:00Z".into(),
        file: Some(dir.join(GROUP_RECORDS_DIR).join("4242.json")),
    }
}

fn assert_run_neutral(text: &str, run: &str) {
    assert!(
        text.starts_with(&format!("run {run} cannot resume")),
        "{text}"
    );
    assert!(!text.contains("fixed decomposition"), "{text}");
    assert!(!text.contains("FixedDecomposition"), "{text}");
}

#[test]
fn unknown_survivor_refusal_names_run_group_and_recorded_pid() {
    let run = tempfile::tempdir().unwrap();
    let refusal = require_no_running_groups_from(
        run.path(),
        "wf-generic",
        vec![record(run.path(), 4243, true)],
        Vec::new(),
    )
    .unwrap_err()
    .to_string();
    assert_run_neutral(&refusal, "wf-generic");
    assert!(refusal.contains("host command 'CHECK-1'"), "{refusal}");
    assert!(refusal.contains("process group 4242"), "{refusal}");
    assert!(refusal.contains("recorded pid 4243"), "{refusal}");
    assert!(refusal.contains("unknown survivors"), "{refusal}");
    assert!(refusal.contains("4242.json"), "{refusal}");
}

#[test]
fn supervised_launch_refusal_names_run_executor_and_barrier() {
    let run = tempfile::tempdir().unwrap();
    let refusal = require_no_running_groups_from(
        run.path(),
        "wf-generic",
        vec![record(run.path(), 0, false)],
        Vec::new(),
    )
    .unwrap_err()
    .to_string();
    assert_run_neutral(&refusal, "wf-generic");
    assert!(
        refusal.contains("launch is still supervised by executor 777"),
        "{refusal}"
    );
    assert!(refusal.contains("(barrier "), "{refusal}");
    assert!(refusal.contains("4242.json"), "{refusal}");
}

#[test]
fn unreadable_membership_refusal_is_run_neutral() {
    let run = tempfile::tempdir().unwrap();
    let refusal =
        remedy::membership_refusal(&record(run.path(), 4243, false), "wf-generic", &"denied");
    assert_run_neutral(&refusal, "wf-generic");
    assert!(
        refusal.contains("host command 'CHECK-1' membership cannot be read (denied)"),
        "{refusal}"
    );
    assert!(refusal.contains("4242.json"), "{refusal}");
}

#[test]
fn read_only_admission_refusal_is_run_neutral_and_keeps_the_record() {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let guard = record_group(&dir, 4242, 4243, None, None, "CHECK-1").unwrap();
    let path = guard.path().to_path_buf();
    guard.keep(None);
    let before = std::fs::read(&path).unwrap();
    let refusal = require_no_running_groups_read_only(run.path(), "wf-generic")
        .unwrap_err()
        .to_string();
    assert_run_neutral(&refusal, "wf-generic");
    assert!(refusal.contains("process group 4242"), "{refusal}");
    assert!(refusal.contains("recorded pid 4243"), "{refusal}");
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
