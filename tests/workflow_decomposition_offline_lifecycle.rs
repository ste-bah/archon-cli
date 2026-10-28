//! Fixed lifecycle baseline and its fault matrix (#257). Set
//! `ARCHON_UPDATE_GOLDEN=1` when running the fresh-run test to rewrite
//! `tests/fixtures/offline_decomposition_golden.json` and
//! `tests/fixtures/offline_decomposition_golden_requests.json`. Every other
//! cell compares against those fixtures.
#[path = "support/workflow_decomposition_canonical_path_test.rs"]
mod canonical_path_test;
#[path = "support/workflow_decomposition_offline_deadline.rs"]
mod deadline;
#[path = "support/workflow_decomposition_offline_diagnostics.rs"]
mod diagnostics;
#[path = "support/workflow_decomposition_offline_faults.rs"]
mod faults;
#[path = "support/workflow_decomposition_offline_launch_repair.rs"]
mod launch_repair;
#[path = "support/workflow_decomposition_offline_project.rs"]
mod project;
#[path = "support/workflow_decomposition_offline_lifecycle_reauth.rs"]
mod reauth_lifecycle;
#[path = "support/workflow_decomposition_offline_server.rs"]
mod server;
#[path = "support/workflow_decomposition_offline_snapshot.rs"]
mod snapshot;
#[path = "support/workflow_decomposition_offline_variant_cells.rs"]
mod variant_cells;

#[path = "support/workflow_decomposition_offline_frozen_repair.rs"]
mod frozen_repair;

use std::fs;

use deadline::{await_provider_boundary, progress, run, tail, wait_for_child};
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

#[cfg(unix)]
#[test]
fn sigstop_over_stream_idle_window_recovers_and_records_the_run() {
    use std::time::Duration;

    const TEST_IDLE_WINDOW_SECS: u64 = 2;
    let project = project::project_with(server::Plan {
        variant: server::Variant::HoldStreamAfterHeaders,
        block: Some(("acceptance".into(), 1)),
    });
    let root = project.path();
    // Exercise the real CLI config surface with a shorter test idle window.
    // This keeps the real child, signals, and wall-clock sleep while avoiding
    // waiting through the production default in this integration test.
    for path in [root.join("config.toml"), root.join(".archon/config.toml")] {
        let config = fs::read_to_string(&path).unwrap();
        let config = config.replace(
            "max_concurrent = 1",
            &format!("max_concurrent = 1\nstream_idle_timeout_secs = {TEST_IDLE_WINDOW_SECS}"),
        );
        assert!(config.contains(&format!(
            "stream_idle_timeout_secs = {TEST_IDLE_WINDOW_SECS}"
        )));
        fs::write(path, config).unwrap();
    }
    let mut child = spawn_decompose(root, &[]);
    await_provider_boundary(&project.provider, root, &mut child, "sigstop-stream");
    let suspended_key = project.provider.requests.snapshot()[0].key.clone();

    let pid = child.id() as libc::pid_t;
    assert_eq!(
        unsafe { libc::kill(pid, libc::SIGSTOP) },
        0,
        "SIGSTOP child"
    );
    std::thread::sleep(Duration::from_secs(TEST_IDLE_WINDOW_SECS + 1));
    assert_eq!(
        unsafe { libc::kill(pid, libc::SIGCONT) },
        0,
        "SIGCONT child"
    );
    project.provider.release();

    let harness = deadline::harness_root(root);
    let stdout = harness.join("decompose.stdout");
    let stderr = harness.join("decompose.stderr");
    let status = wait_for_child(&mut child, &stdout, &stderr, "sigstop-stream");
    assert!(
        status.success(),
        "stdout: {}; stderr: {}",
        tail(&stdout),
        tail(&stderr)
    );

    let requests = project.provider.requests.snapshot();
    let recovery_requests = requests
        .iter()
        .filter(|request| request.key == suspended_key)
        .count();
    assert_eq!(
        recovery_requests, 2,
        "one provider error and one resend: {requests:?}"
    );
    let run_dir = one_run(root);
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("state.json")).unwrap()).unwrap();
    println!(
        "EVIDENCE #364 sigstop-recovery BEFORE: child suspended with open stream AFTER: state={} same-call-requests={recovery_requests}",
        state["status"]
    );
    assert_eq!(state["status"], "completed", "durable state: {state}");
    assert!(root.join("tasks/PRD-X/acceptance-contract.json").exists());
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
            &deadline::harness_root(root).join("decompose.stdout"),
            &deadline::harness_root(root).join("decompose.stderr"),
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

#[test]
fn reasoning_bank_rows_are_written_at_pause_and_remain_after_resume() {
    let project = project_with_provider(Some("skeleton"));
    let root = project.path();
    for path in [root.join("config.toml"), root.join(".archon/config.toml")] {
        let mut config = fs::read_to_string(&path).unwrap();
        config.push_str(
            "\n[learning.reasoning_bank]\nenabled = true\n[learning.desc]\nenabled = true\n",
        );
        fs::write(path, config).unwrap();
    }

    let mut child = spawn_decompose(root, &[]);
    await_provider_boundary(&project.provider, root, &mut child, "reasoning-bank pause");
    let run_dir = one_run(root);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().into_owned();
    let launch_state: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("decomposition/state.json")).unwrap())
            .unwrap();
    let launch_script_digest = launch_state["identity"]["script_digest"].clone();
    assert!(launch_script_digest.as_str().is_some());
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
            &deadline::harness_root(root).join("decompose.stdout"),
            &deadline::harness_root(root).join("decompose.stderr"),
            "reasoning-bank pause",
        )
        .success()
    );

    let paused = read_reasoning_patterns(root);
    assert!(
        !paused.is_empty(),
        "pause fold persisted no ReasoningBank rows"
    );
    let paused_state: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("decomposition/state.json")).unwrap())
            .unwrap();
    let script_digest = paused_state["identity"]["script_digest"].clone();
    assert_eq!(script_digest, launch_script_digest);
    let resumed = resume(root, &run_id);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let ended = read_reasoning_patterns(root);
    assert!(
        paused.iter().all(|row| ended.contains(row)),
        "resume changed or removed a paused ReasoningBank row"
    );
    assert!(
        ended.len() > paused.len(),
        "end fold added no ReasoningBank lessons"
    );
    let ended_ids: std::collections::HashSet<_> = ended.iter().map(|(id, _)| id).collect();
    assert_eq!(
        ended_ids.len(),
        ended.len(),
        "end fold wrote duplicate ReasoningBank row ids"
    );
    let ended_state: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("state.json")).unwrap()).unwrap();
    let ended_decomposition_state: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("decomposition/state.json")).unwrap())
            .unwrap();
    assert_eq!(
        ended_state["status"], "completed",
        "resumed run must end: {ended_state}"
    );
    assert_eq!(
        ended_decomposition_state["identity"]["script_digest"],
        script_digest
    );
    let interrupted_key = project
        .provider
        .requests
        .snapshot()
        .into_iter()
        .filter(|request| request.role == "skeleton")
        .nth(0)
        .expect("the interrupted skeleton request is logged")
        .key;
    let mut expected_requests = project::read_golden_requests();
    *expected_requests.entry(interrupted_key).or_default() += 1;
    assert_eq!(
        project.provider.requests.per_call(),
        expected_requests,
        "resumed provider requests per call differ from golden (+1 only for the interrupted request)"
    );
}

fn read_reasoning_patterns(root: &std::path::Path) -> Vec<(String, String)> {
    let path = root
        .join(".archon/learning-state.db")
        .to_string_lossy()
        .to_string();
    let db = archon_cozo::open_sqlite_guarded_instance(
        &path,
        "read lifecycle ReasoningBank rows",
        archon_cozo::CozoGuardConfig::for_db_path(&path),
    )
    .unwrap()
    .db_arc();
    db.run_script(
        "?[pattern_id, description] := *patterns{pattern_id, description}",
        Default::default(),
        cozo::ScriptMutability::Immutable,
    )
    .unwrap()
    .rows
    .iter()
    .map(|row| {
        (
            row[0].get_str().unwrap().to_string(),
            row[1].get_str().unwrap().to_string(),
        )
    })
    .collect()
}
