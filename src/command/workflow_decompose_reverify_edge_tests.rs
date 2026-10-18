//! Refusal and publication edge cases for reverify.
use super::workflow_decompose_reverify_tests::{archon_binary, fixture, invoke_reverify};
use archon_workflow::RunStatus;
use std::path::Path;

pub(super) fn assert_binary_identity(binary: &Path) {
    let output = std::process::Command::new(binary)
        .arg("--build-fingerprint")
        .output()
        .unwrap_or_else(|error| panic!("could not inspect archon build identity at {}: {error}; run `cargo build --bin archon`", binary.display()));
    assert!(
        output.status.success(),
        "could not inspect archon build identity at {}; run `cargo build --bin archon`",
        binary.display()
    );
    let actual = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    assert_build_identity(env!("ARCHON_BUILD_FINGERPRINT"), &actual);
}

fn assert_build_identity(expected: &str, actual: &str) {
    assert_eq!(
        actual, expected,
        "archon executable has a different build identity; run `cargo build --bin archon`"
    );
}

#[test]
fn archon_binary_rejects_a_different_build_identity() {
    let result = std::panic::catch_unwind(|| assert_build_identity("current-build", "old-build"));
    assert!(result.is_err());
}

#[cfg(unix)]
#[test]
fn archon_binary_check_rejects_a_mismatched_executable() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("wrong-archon");
    std::fs::write(&script, "#!/bin/sh\nprintf 'different-fingerprint\\n'\n").unwrap();
    let mut permissions = std::fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).unwrap();

    let result = std::panic::catch_unwind(|| assert_binary_identity(&script));
    assert!(result.is_err(), "mismatched executable should be rejected");
}

#[test]
fn current_archon_binary_passes_the_identity_check() {
    assert_binary_identity(&archon_binary());
}

#[tokio::test]
async fn refusals_for_wrong_kind_or_incomplete_run_do_not_write_results() {
    let binary = archon_binary();
    for wrong_kind in [false, true] {
        let fixture = fixture();
        let state_path = fixture
            .store
            .run_dir(&fixture.run_id)
            .join("decomposition/state.json");
        let mut decomposition: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
        if wrong_kind {
            decomposition["run_kind"] = serde_json::json!("legacy_decomposed");
            std::fs::write(
                &state_path,
                serde_json::to_vec_pretty(&decomposition).unwrap(),
            )
            .unwrap();
        } else {
            let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
            run.status = RunStatus::Paused;
            fixture.store.save_state(&run).unwrap();
        }
        let before_run = std::fs::read(fixture.store.state_path(&fixture.run_id)).unwrap();
        let before_decomposition = std::fs::read(&state_path).unwrap();
        let before_bodies = fixture
            .bodies
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect::<Vec<_>>();
        let reports = fixture
            .store
            .run_dir(&fixture.run_id)
            .join("decomposition/reverify/reports");
        println!(
            "EVIDENCE reverify refusal BEFORE: kind={} status={:?} reports_present={}",
            if wrong_kind { "wrong" } else { "fixed" },
            fixture.store.load_state(&fixture.run_id).unwrap().status,
            reports.exists()
        );
        let output = invoke_reverify(&binary, &fixture);
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        if wrong_kind {
            assert!(
                stderr.contains("not a completed fixed decomposition"),
                "{stderr}"
            );
        } else {
            assert!(stderr.contains("is not completed"), "{stderr}");
        }
        assert_eq!(
            std::fs::read(fixture.store.state_path(&fixture.run_id)).unwrap(),
            before_run
        );
        assert_eq!(std::fs::read(&state_path).unwrap(), before_decomposition);
        assert_eq!(
            fixture
                .bodies
                .iter()
                .map(|path| std::fs::read(path).unwrap())
                .collect::<Vec<_>>(),
            before_bodies
        );
        assert!(!reports.exists());
        println!(
            "EVIDENCE reverify refusal AFTER: refused=true run_bytes_unchanged=true state_bytes_unchanged=true bodies_unchanged=true reports_present={}",
            reports.exists()
        );
    }
}

#[tokio::test]
async fn live_executor_lease_refuses_reverify_without_writing_results() {
    let fixture = fixture();
    let binary = archon_binary();
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    let lease_path = run_dir.join(crate::command::workflow_executor_lease::LEASE);
    let lease =
        crate::command::workflow_executor_lease::acquire(&run_dir, &fixture.run_id).unwrap();
    let run_bytes = std::fs::read(fixture.store.state_path(&fixture.run_id)).unwrap();
    let body_bytes = fixture
        .bodies
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect::<Vec<_>>();
    let reports = run_dir.join("decomposition/reverify/reports");
    println!(
        "EVIDENCE reverify live_lease BEFORE: held=true status=Completed reports_present={}",
        reports.exists()
    );
    let output = invoke_reverify(&binary, &fixture);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("is live"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read(fixture.store.state_path(&fixture.run_id)).unwrap(),
        run_bytes
    );
    assert_eq!(
        fixture
            .bodies
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect::<Vec<_>>(),
        body_bytes
    );
    assert!(!reports.exists());
    println!(
        "EVIDENCE reverify live_lease AFTER: refused=true run_unchanged=true bodies_unchanged=true reports_present={}",
        reports.exists()
    );
    drop(lease);
    let lease_record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&lease_path).unwrap()).unwrap();
    assert_eq!(lease_record["role"], "preflight");
    assert_eq!(lease_record["holder"]["pid"], std::process::id());
    println!(
        "EVIDENCE reverify live_lease SOURCE: lease file read after handle release; role={} pid_matches=true",
        lease_record["role"]
    );
}

#[tokio::test]
async fn pending_publish_journal_refuses_reverify_without_writing_results() {
    let fixture = fixture();
    let binary = archon_binary();
    let task_root = fixture.project.join("tasks/PRD-X");
    let pin = crate::command::workflow_task_set::acceptance_pin_path(&fixture.project, &task_root);
    let [journal, _] = archon_workflow::task_set_publish_lock::journal_paths(&pin);
    std::fs::write(&journal, b"{}").unwrap();
    let journal_before = std::fs::read(&journal).unwrap();
    let run_bytes = std::fs::read(fixture.store.state_path(&fixture.run_id)).unwrap();
    let body_bytes = fixture
        .bodies
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect::<Vec<_>>();
    let reports = fixture
        .store
        .run_dir(&fixture.run_id)
        .join("decomposition/reverify/reports");
    println!(
        "EVIDENCE reverify pending_journal BEFORE: journal_present=true run=Completed reports_present={}",
        reports.exists()
    );
    let output = invoke_reverify(&binary, &fixture);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("publish recovery is pending"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(&journal).unwrap(), journal_before);
    assert_eq!(
        std::fs::read(fixture.store.state_path(&fixture.run_id)).unwrap(),
        run_bytes
    );
    assert_eq!(
        fixture
            .bodies
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect::<Vec<_>>(),
        body_bytes
    );
    assert!(!reports.exists());
    println!(
        "EVIDENCE reverify pending_journal AFTER: refused=true journal_unchanged=true run_unchanged=true bodies_unchanged=true reports_present={}",
        reports.exists()
    );
}
