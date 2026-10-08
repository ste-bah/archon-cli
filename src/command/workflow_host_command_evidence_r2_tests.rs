use super::*;

fn before_invalidation_failure(phase: &str) {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(GROUP_RECORDS_DIR);
    let id = ended_group();
    let guard = record_group(&dir, id, id, None, None, "cmd").unwrap();
    let path = guard.path().to_path_buf();
    let pending = path.with_extension("pending");
    let mut record = guard::pending_record(&pending).unwrap().unwrap().record;
    record.host_pid = ended_group();
    record.host_start = None;
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    std::fs::write(&pending, guard::supervised_marker(&record).unwrap()).unwrap();
    // A read-only HANDLE deterministically rejects set_len before any bytes
    // change, modelling a filesystem turning read-only after registration.
    let evidence = GroupEvidence::new(std::fs::File::open(&pending).unwrap(), record);
    let failure = match phase {
        "scan" => evidence.remember(&[(std::process::id(), owner::own_start().unwrap())]),
        "freeze" => evidence.begin(),
        _ => evidence.complete(&[]),
    };
    assert!(failure.is_err());
    owner::release(&pending); // the executor crashed before keep()
    let refused = require_no_running_groups(run.path(), "run");
    drop(guard);
    assert!(
        refused.is_err(),
        "{phase}: old evidence falsely healed after a write failed before invalidation"
    );
}

#[test]
fn scan_failure_before_invalidation_survives_executor_crash() {
    before_invalidation_failure("scan");
}
#[test]
fn freeze_failure_before_invalidation_survives_executor_crash() {
    before_invalidation_failure("freeze");
}
#[test]
fn completion_failure_before_invalidation_survives_executor_crash() {
    before_invalidation_failure("complete");
}

#[test]
fn known_escaped_survivors_have_an_identity_remedy() {
    for launch in 0..3 {
        let run = tempfile::tempdir().unwrap();
        let guard = record_group(
            &run.path().join(GROUP_RECORDS_DIR),
            std::process::id(),
            0,
            None,
            None,
            "cmd",
        )
        .unwrap();
        assert!(
            stalled_running(run.path()).unwrap().is_empty(),
            "a progressing launch {launch} was treated as a stall"
        );
        let note = require_no_running_groups(run.path(), "run")
            .unwrap_err()
            .to_string();
        assert!(
            note.contains("wait for that executor")
                && note.contains(&guard.path().display().to_string()),
            "{note}"
        );
        assert!(
            !note.contains("kill -TERM"),
            "a launch record does not name a child group: {note}"
        );
    }
    for session in [None, Some(ended_group()), Some(std::process::id())] {
        let run = tempfile::tempdir().unwrap();
        let (mut child, identity) = sleeper();
        let id = ended_group();
        record_group(
            &run.path().join(GROUP_RECORDS_DIR),
            id,
            id,
            session,
            None,
            "cmd",
        )
        .unwrap()
        .keep(Some(&[identity]));
        let note = require_no_running_groups(run.path(), "run")
            .unwrap_err()
            .to_string();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(
            note.contains(&format!("kill -TERM {}", identity.0)),
            "{note}"
        );
        assert!(
            note.contains(&identity.1.to_string()),
            "must verify the creation time: {note}"
        );
        assert!(!note.contains(&format!("kill -TERM -{id}")), "{note}");
    }
}

#[test]
fn crash_fixture_uses_shared_spawn() {
    assert!(
        !include_str!("workflow_host_command_groups_regression_tests.rs")
            .contains("std::process::Command::new")
    );
}
#[test]
fn windows_record_fixtures_use_shared_spawn() {
    assert!(
        !include_str!("../../crates/archon-shell/src/job_object_tests.rs")
            .contains("std::process::Command::new")
    );
    assert!(
        !include_str!("workflow_host_command_groups_windows_tests.rs")
            .contains("std::process::Command::new")
    );
}
#[test]
fn windows_scratch_fixtures_use_shared_spawn() {
    assert!(
        !include_str!(
            "../../crates/archon-workflow/src/acceptance_scratch_process_windows_tests.rs"
        )
        .contains("tokio::process::Command::new")
    );
}

#[path = "workflow_host_spawn_r3_tests.rs"]
mod spawn_r3_tests;
