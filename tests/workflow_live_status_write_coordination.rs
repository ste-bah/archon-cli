//! TASK-WC-009 — workspace-level integration test (AC-WC-009): the live
//! write-coordination status block renders from persisted coordinator state.

use std::process::Command;

use archon_workflow::WorkflowStore;
use archon_workflow::write_coordinator::patch_apply::{ApplyRecord, VerifyResult};
use archon_workflow::write_coordinator::status::{
    coordinated_stage_ids, read_status, render_compact,
};

fn git(args: &[&str], cwd: &std::path::Path) {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Seed a persisted apply record under the store run dir and assert the compact
/// status block renders the §17 format.
#[test]
fn workflow_live_status_write_coordination_renders() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(dir.path());
    let run_id = "run-live";
    let apply_dir = store
        .run_dir(run_id)
        .join("write-coordination/stages/implement/apply");
    std::fs::create_dir_all(&apply_dir).unwrap();

    let record = ApplyRecord {
        wave_id: 0,
        started_at: std::time::SystemTime::UNIX_EPOCH,
        completed_at: std::time::SystemTime::UNIX_EPOCH,
        items_applied: vec!["implement-0".into(), "implement-1".into()],
        items_failed: vec![],
        verify_result: Some(VerifyResult {
            exit: 0,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            duration_ms: 1,
        }),
    };
    std::fs::write(
        apply_dir.join("0.json"),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();

    let stages = coordinated_stage_ids(&store, run_id);
    assert!(stages.contains(&"implement".to_string()));

    let status = read_status(&store, run_id, "implement")
        .unwrap()
        .expect("status");
    let block = render_compact(&status);
    assert_eq!(block.lines().count(), 6, "got: {block}");
    assert!(block.starts_with("write_coordination: enabled\n"));
    assert!(block.contains("items: 0 running, 0 failed, 2 accepted"));
    assert!(block.contains("apply: applied"));

    // git helper kept honest for parity with other workspace tests.
    let repo = tempfile::tempdir().unwrap();
    git(&["init", "-q"], repo.path());
}
