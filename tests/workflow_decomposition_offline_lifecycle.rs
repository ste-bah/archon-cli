#[path = "support/workflow_decomposition_offline_deadline.rs"]
mod deadline;
#[path = "support/workflow_decomposition_offline_diagnostics.rs"]
mod diagnostics;
#[path = "support/workflow_decomposition_offline_server.rs"]
mod server;
#[path = "support/workflow_decomposition_offline_snapshot.rs"]
mod snapshot;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use deadline::{await_provider_boundary, isolate_child, progress, run, wait_for_child};
use snapshot::{result_store_records, strip_timestamps};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/offline_decomposition"
);

struct TestProject {
    temp: Option<tempfile::TempDir>,
    provider: server::FakeProvider,
}

impl TestProject {
    fn path(&self) -> &Path {
        self.temp.as_ref().unwrap().path()
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        let root = self.path().to_path_buf();
        diagnostics::print_failure(&root, &self.provider.requests);
        if let Some(temp) = self.temp.take() {
            let _ = temp.keep();
        }
    }
}

fn project() -> TestProject {
    project_with_provider(None)
}

fn project_with_provider(blocked_role: Option<&str>) -> TestProject {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("repository")).unwrap();
    fs::create_dir_all(root.join("tasks")).unwrap();
    copy_tree(
        &PathBuf::from(FIXTURE).join("repository"),
        &root.join("repository"),
    );
    copy_tree(
        &PathBuf::from(FIXTURE).join(".archon"),
        &root.join(".archon"),
    );
    fs::copy(PathBuf::from(FIXTURE).join("prd.md"), root.join("prd.md")).unwrap();
    let git = Command::new("git")
        .current_dir(root.join("repository"))
        .args(["init", "-q"])
        .status()
        .unwrap();
    assert!(git.success());
    let git = Command::new("git")
        .current_dir(root.join("repository"))
        .args(["add", "."])
        .status()
        .unwrap();
    assert!(git.success());
    let git = Command::new("git")
        .current_dir(root.join("repository"))
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "fixture",
        ])
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .status()
        .unwrap();
    assert!(git.success());
    let provider = blocked_role.map_or_else(
        server::FakeProvider::start,
        server::FakeProvider::start_blocked,
    );
    let config = format!(
        r#"
[api]
default_model = "claude-sonnet-4-6"
default_effort = "low"
thinking_budget = 0
base_url = "{}"
auth = "required"
[llm]
provider = "anthropic"
[llm.openai]
base_url = "{}"
[llm.local]
base_url = "{}"
[workflow]
gate_mode = "observe"
repository_root = "repository"
[subagent]
max_concurrent = 1
"#,
        provider.base_url,
        provider.base_url.trim_end_matches("/v1/messages"),
        provider.base_url.trim_end_matches("/v1/messages")
    );
    fs::write(root.join("config.toml"), &config).unwrap();
    fs::write(root.join(".archon/config.toml"), &config).unwrap();
    TestProject {
        temp: Some(temp),
        provider,
    }
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let destination = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}

fn decompose_args() -> [&'static str; 9] {
    [
        "workflow",
        "decompose",
        "--prd",
        "prd.md",
        "--tasks",
        "tasks/PRD-X",
        "--repository",
        "repository",
        "--yes",
    ]
}

fn spawn_decompose(root: &Path) -> std::process::Child {
    spawn_decompose_with_revision(root, None)
}

fn spawn_decompose_with_revision(root: &Path, revision: Option<&str>) -> std::process::Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_archon"));
    isolate_child(&mut command, root);
    command
        .current_dir(root)
        .args(decompose_args())
        .envs(
            revision
                .map(|revision| [("ARCHON_GIT_HASH", revision)])
                .into_iter()
                .flatten(),
        )
        .stdout(fs::File::create(root.join("decompose.stdout")).unwrap())
        .stderr(fs::File::create(root.join("decompose.stderr")).unwrap())
        .spawn()
        .unwrap()
}

fn accepted_call_counts(root: &Path) -> std::collections::BTreeMap<String, usize> {
    let events = fs::read_to_string(one_run(root).join("events.jsonl")).unwrap();
    let mut counts = std::collections::BTreeMap::new();
    for line in events.lines() {
        let event: serde_json::Value = serde_json::from_str(line).unwrap();
        if event["kind"] == "author_attempt_completed"
            && event["detail"]["status"] == "accepted"
            && let Some(call_id) = event["detail"]["call_id"].as_str()
        {
            *counts.entry(call_id.to_string()).or_default() += 1;
        }
    }
    counts
}

fn assert_only_call_id_accepted_again(
    before: &std::collections::BTreeMap<String, usize>,
    after: &std::collections::BTreeMap<String, usize>,
    interrupted_call_id: &str,
) {
    for (call_id, prior_count) in before {
        let expected =
            prior_count + usize::from(call_id == interrupted_call_id && *prior_count == 0);
        let after_count = after.get(call_id).copied().unwrap_or_default();
        assert_eq!(
            after_count, expected,
            "unexpected accepted call replay for {call_id}: before={before:?} after={after:?}"
        );
    }
    if !before.contains_key(interrupted_call_id) {
        assert_eq!(
            after.get(interrupted_call_id).copied().unwrap_or_default(),
            1,
            "the interrupted call must be accepted once after resume: before={before:?} after={after:?}"
        );
    }
}

fn resume(root: &Path, run_id: &str) -> Output {
    run(root, &["workflow", "resume", "--live", "--yes", run_id])
}

fn one_run(root: &Path) -> PathBuf {
    let runs = fs::read_dir(root.join(".archon/workflows")).unwrap();
    let mut matches = runs
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("state.json").is_file());
    let run = matches.next().expect("decomposition result-store run");
    assert!(matches.next().is_none(), "fixture should create one run");
    run
}

#[derive(Debug, PartialEq)]
struct FinalSnapshot {
    acceptance: serde_json::Value,
    skeleton: serde_json::Value,
    task_bodies: Vec<(String, String)>,
    records: Vec<(String, serde_json::Value)>,
}

fn final_snapshot(project: &TestProject) -> FinalSnapshot {
    let root = project.path();
    let run = one_run(root);
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("state.json")).unwrap()).unwrap();
    assert_eq!(state["status"], "completed", "final run state: {state}");
    let mut acceptance: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("tasks/PRD-X/acceptance-contract.json")).unwrap(),
    )
    .unwrap();
    strip_timestamps(&mut acceptance);
    if let Some(prd) = acceptance
        .get_mut("prd")
        .and_then(serde_json::Value::as_object_mut)
    {
        prd.remove("path");
    }
    let mut skeleton: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("tasks/PRD-X/task-skeleton.json")).unwrap())
            .unwrap();
    strip_timestamps(&mut skeleton);
    let task_bodies = ["TASK-X-010.md", "TASK-X-020.md"]
        .into_iter()
        .map(|name| {
            (
                name.into(),
                fs::read_to_string(root.join("tasks/PRD-X").join(name)).unwrap(),
            )
        })
        .collect();
    let records = result_store_records(
        &run,
        root,
        &project.provider.base_url,
        run.file_name().unwrap().to_str().unwrap(),
    );
    assert!(
        !records.is_empty(),
        "the result store must retain host-call records"
    );
    FinalSnapshot {
        acceptance,
        skeleton,
        task_bodies,
        records,
    }
}

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
    let _golden = final_snapshot(&project);
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
    for role in ["acceptance", "judge", "skeleton", "body", "audit"] {
        assert!(
            counts.get(role).copied().unwrap_or_default() > 0,
            "missing scripted {role} call: {counts:?}"
        );
    }
    assert_eq!(
        counts.get("unexpected"),
        None,
        "every model request must match an explicit script: {counts:?}"
    );
}

#[test]
fn pause_at_provider_call_boundary_resumes_only_interrupted_author() {
    progress("pause-resume", "starting blocked provider call");
    let project = project_with_provider(Some("skeleton"));
    let root = project.path();
    let mut child = spawn_decompose(root);
    await_provider_boundary(&project.provider, root, &mut child, "pause-resume cell");
    progress("pause-resume", "provider call reached pause boundary");
    let run_dir = one_run(root);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().into_owned();
    let pause = run(root, &["workflow", "pause", &run_id]);
    assert!(
        pause.status.success(),
        "{}",
        String::from_utf8_lossy(&pause.stderr)
    );
    project.provider.release();
    let child_status = wait_for_child(
        &mut child,
        &root.join("decompose.stdout"),
        &root.join("decompose.stderr"),
        "paused decomposition child",
    );
    assert!(
        child_status.success(),
        "paused child should settle cleanly: {child_status}"
    );
    let accepted_before_resume = accepted_call_counts(root);
    progress("pause-resume", "resuming interrupted author");
    let resumed = resume(root, &run_id);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_only_call_id_accepted_again(
        &accepted_before_resume,
        &accepted_call_counts(root),
        "skeleton-author-1",
    );
    let actual = final_snapshot(&project);
    let golden = completed_golden_snapshot();
    assert_eq!(
        actual, golden,
        "pause and resume must preserve the golden result-store records and artifacts"
    );
}

#[cfg(unix)]
#[test]
fn sigkill_mid_provider_call_resumes_only_interrupted_author() {
    progress("sigkill-resume", "starting blocked provider call");
    let project = project_with_provider(Some("skeleton"));
    let root = project.path();
    let mut child = spawn_decompose(root);
    await_provider_boundary(&project.provider, root, &mut child, "sigkill-resume cell");
    let run_dir = one_run(root);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().into_owned();
    child.kill().unwrap();
    let _ = wait_for_child(
        &mut child,
        &root.join("decompose.stdout"),
        &root.join("decompose.stderr"),
        "killed decomposition child",
    );
    let accepted_before_resume = accepted_call_counts(root);
    project.provider.release();
    let resumed = resume(root, &run_id);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_only_call_id_accepted_again(
        &accepted_before_resume,
        &accepted_call_counts(root),
        "skeleton-author-1",
    );
    assert_eq!(
        final_snapshot(&project),
        completed_golden_snapshot(),
        "SIGKILL recovery must preserve golden records and artifacts"
    );
}

#[test]
#[ignore = "defect: the real CLI has no process-level build-id override; ARCHON_GIT_HASH is compile-time only"]
fn resume_from_a_different_build_id_preserves_the_golden_result() {
    let project = project_with_provider(Some("skeleton"));
    let root = project.path();
    let mut child = spawn_decompose_with_revision(root, Some("previous-build"));
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
    assert_eq!(final_snapshot(&project), completed_golden_snapshot());
}

fn completed_golden_snapshot() -> FinalSnapshot {
    let project = project();
    let output = run(project.path(), &decompose_args());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    final_snapshot(&project)
}
