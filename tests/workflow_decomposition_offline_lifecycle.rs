//! Fixed lifecycle baseline and its fault matrix (#257). Set
//! `ARCHON_UPDATE_GOLDEN=1` when running the fresh-run test to rewrite
//! `tests/fixtures/offline_decomposition_golden.json` and
//! `tests/fixtures/offline_decomposition_golden_requests.json`. Every other
//! cell compares against those fixtures.
#[path = "support/workflow_decomposition_offline_deadline.rs"]
mod deadline;
#[path = "support/workflow_decomposition_offline_diagnostics.rs"]
mod diagnostics;
#[path = "support/workflow_decomposition_offline_faults.rs"]
mod faults;
#[path = "support/workflow_decomposition_offline_project.rs"]
mod project;
#[path = "support/workflow_decomposition_offline_server.rs"]
mod server;
#[path = "support/workflow_decomposition_offline_snapshot.rs"]
mod snapshot;
#[path = "support/workflow_decomposition_offline_variant_cells.rs"]
mod variant_cells;

use std::fs;

use deadline::{await_provider_boundary, progress, run, wait_for_child};
use project::{
    assert_golden_requests, assert_no_unknown_prompts, decompose_args, final_snapshot, one_run,
    project, project_with_provider, resume, spawn_decompose,
};

#[test]
fn golden_live_decomposition_uses_real_binary_and_local_provider() {
    progress("golden", "starting local provider and CLI");
    let project = project();
    let root = project.path();
    let output = run(root, &decompose_args());
    assert!(
        output.status.success(),
        "exit status: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run_dir = one_run(root);
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("state.json")).unwrap()).unwrap();
    assert_eq!(
        state["status"],
        "completed",
        "CLI output: {}\nstate: {state}",
        String::from_utf8_lossy(&output.stdout)
    );
    progress("golden", "CLI completed; checking artifacts and calls");
    assert!(
        root.join("tasks/PRD-X/acceptance-contract.json").exists(),
        "CLI output: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(root.join("tasks/PRD-X/task-skeleton.json").exists());
    assert!(root.join("tasks/PRD-X/TASK-X-010.md").exists());
    assert!(root.join("tasks/PRD-X/TASK-X-020.md").exists());
    let actual = final_snapshot(&project, &[]);
    snapshot::assert_golden_snapshot(&serde_json::to_value(&actual).unwrap());
    let counts = project.provider.counts.snapshot();
    let requests = project.provider.requests.snapshot();
    assert!(
        requests.iter().all(|request| request.known_path),
        "unknown fake-provider path: {requests:?}"
    );
    assert!(
        requests.iter().all(|request| request.auth_present),
        "a fake-provider request omitted authentication: {requests:?}"
    );
    assert!(
        counts.get("acceptance").copied().unwrap_or_default() > 0,
        "acceptance author must reach the configured local provider; requests: {:?}",
        project.provider.requests.snapshot()
    );
    for role in ["acceptance", "judge", "skeleton", "body", "fidelity"] {
        assert!(
            counts.get(role).copied().unwrap_or_default() > 0,
            "missing scripted {role} call: {counts:?}"
        );
    }
    assert_no_unknown_prompts(&project.provider);
    assert_golden_requests(&project.provider.requests.per_call());
    progress("golden", "passed");
}

#[test]
fn resume_from_a_different_build_id_preserves_the_golden_result() {
    progress("different-build", "starting");
    let project = project_with_provider(Some("skeleton"));
    let root = project.path();
    // The launch records `previous-build` as its build id through the
    // debug-build seam (src/command/workflow_test_seams.rs); the resume runs
    // as the real build.
    let mut child = spawn_decompose(
        root,
        &[("ARCHON_TEST_BINARY_REVISION", "previous-build".to_string())],
    );
    await_provider_boundary(&project.provider, root, &mut child, "different-build cell");
    let run_dir = one_run(root);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().into_owned();
    let pause = run(root, &["workflow", "pause", &run_id]);
    assert!(
        pause.status.success(),
        "{}",
        String::from_utf8_lossy(&pause.stderr)
    );
    project.provider.release();
    assert!(
        wait_for_child(
            &mut child,
            &root.join("decompose.stdout"),
            &root.join("decompose.stderr"),
            "different-build decomposition child",
        )
        .success()
    );
    let resumed = resume(root, &run_id);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let events = fs::read_to_string(run_dir.join("events.jsonl")).unwrap();
    assert!(
        events.contains("binary_revision_drift"),
        "resume must observe the different starting build: {events}"
    );
    let mut golden: project::FinalSnapshot =
        serde_json::from_value(snapshot::read_golden_snapshot()).unwrap();
    snapshot::golden_records_for(&mut golden.records, &["skeleton-author-"]);
    project::assert_same_json(
        "different build id: final artifacts and records against golden",
        &project::keyed(
            serde_json::to_value(final_snapshot(&project, &["skeleton-author-"])).unwrap(),
        ),
        &project::keyed(serde_json::to_value(golden).unwrap()),
    );
    progress("different-build", "passed");
}
