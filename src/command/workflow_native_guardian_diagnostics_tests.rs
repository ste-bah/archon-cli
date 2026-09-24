//! A failing guardian must be diagnosable, and one unauthorized check must not
//! stand in for the whole gate.
use super::*;
use crate::command::acceptance_scratch_guardian::diagnostics::{
    child_environment, failure_context,
};
use archon_workflow::acceptance_scratch::ScratchPolicy;

fn policy(
    repository: &std::path::Path,
    project: &std::path::Path,
    task_root: &std::path::Path,
    scratch_parent: &std::path::Path,
    toolchain: &str,
    allowlist: Vec<String>,
) -> ScratchPolicy {
    ScratchPolicy {
        repository: repository.into(),
        project: project.into(),
        task_root: task_root.into(),
        scratch_parent: scratch_parent.into(),
        project_inputs: vec![],
        project_input_excludes: vec![],
        combined: true,
        toolchain_path: toolchain.into(),
        environment: Default::default(),
        environment_allowlist: allowlist,
        cargo_seed: None,
        timeout_secs: 60,
        output_bytes: 4096,
        scratch_bytes: 16777216,
    }
}

/// Commits one file and returns the repository and its HEAD.
fn source_repository() -> (tempfile::TempDir, String) {
    let repo = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
    git(&["config", "user.name", "fixture"]);
    std::fs::write(repo.path().join("input"), "present").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "source"]);
    let head = git(&["rev-parse", "HEAD"]);
    (repo, head)
}

fn request(
    fixture: &FrozenFixture,
    policy: ScratchPolicy,
    source_commit: String,
    evidence: std::path::PathBuf,
) -> crate::command::acceptance_scratch_guardian::Request {
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(
        fixture.project.path(),
        &fixture.task_root,
    );
    crate::command::acceptance_scratch_guardian::Request {
        policy,
        source_commit,
        expected_pin_digest: content_digest(&std::fs::read(&pin_path).unwrap()),
        pin_path,
        evidence,
    }
}

#[test]
fn child_environment_derives_path_from_the_toolchain_and_clears_everything_else() {
    let root = tempfile::tempdir().unwrap();
    let policy = policy(
        root.path(),
        root.path(),
        root.path(),
        root.path(),
        "/fixture/toolchain/bin:/usr/bin",
        vec!["FIXTURE_ALLOWED".into(), "FIXTURE_ABSENT".into()],
    );
    let host = |key: &str| match key {
        "FIXTURE_ALLOWED" => Some(std::ffi::OsString::from("allowed-value")),
        "FIXTURE_DENIED" => Some(std::ffi::OsString::from("denied-value")),
        _ => None,
    };
    let bindings = child_environment(&policy, host);
    assert_eq!(
        bindings,
        vec![
            (
                "PATH".to_string(),
                std::ffi::OsString::from("/fixture/toolchain/bin:/usr/bin")
            ),
            (
                "FIXTURE_ALLOWED".to_string(),
                std::ffi::OsString::from("allowed-value")
            ),
        ],
        "PATH must come from the configured toolchain, an allowlisted key that \
         is set must pass through, and nothing else may cross the boundary"
    );
}

#[test]
fn failure_context_refuses_to_name_an_evidence_directory_that_does_not_exist() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("never-written");
    let absent = failure_context(&missing, "child said why");
    assert!(
        absent.contains("no evidence directory was written"),
        "{absent}"
    );
    assert!(absent.contains("child said why"), "{absent}");
    std::fs::create_dir(&missing).unwrap();
    let present = failure_context(&missing, "");
    assert!(present.contains("evidence: "), "{present}");
    assert!(!present.contains("no evidence directory"), "{present}");
    assert!(present.contains("nothing to stderr"), "{present}");
}

#[tokio::test]
async fn guardian_failure_carries_child_stderr_and_reports_the_missing_evidence() {
    let fixture = frozen_fixture(vec![criterion("AC-X-001", command("test -f input".into()))]);
    let (repo, head) = source_repository();
    let scratch = tempfile::tempdir().unwrap();
    // Refused by the child only: an evidence path inside a live root. The
    // child exits before it can create the directory the parent names.
    let evidence = fixture.project.path().join("inside-live-root");
    let error = crate::command::acceptance_scratch_guardian::launch(request(
        &fixture,
        policy(
            repo.path(),
            fixture.project.path(),
            &fixture.task_root,
            scratch.path(),
            "/usr/bin:/bin",
            vec![],
        ),
        head,
        evidence.clone(),
    ))
    .await
    .unwrap_err()
    .to_string();
    assert!(!evidence.exists(), "the child never wrote this directory");
    assert!(
        error.contains("provisional evidence must be outside live roots"),
        "the child's own stderr must reach the stage error: {error}"
    );
    assert!(
        error.contains("no evidence directory was written"),
        "the error must not send an operator to a path nothing wrote: {error}"
    );
}

#[tokio::test]
async fn one_unauthorized_check_does_not_stop_the_rest_of_the_gate() {
    let mut refuted = criterion("AC-X-002", command("test -f input".into()));
    refuted.judgment.verdict = JudgeDecision::Refuted;
    let fixture = frozen_fixture(vec![
        criterion("AC-X-001", command("test -f input".into())),
        refuted,
    ]);
    let (repo, head) = source_repository();
    let scratch = tempfile::tempdir().unwrap();
    let evidence = scratch.path().join("evidence");
    let result = crate::command::acceptance_scratch_guardian::launch(request(
        &fixture,
        policy(
            repo.path(),
            fixture.project.path(),
            &fixture.task_root,
            scratch.path(),
            "/usr/bin:/bin",
            vec![],
        ),
        head,
        evidence.clone(),
    ))
    .await
    .unwrap();
    assert!(
        evidence.join("observation.json").is_file(),
        "the observation must still publish its evidence"
    );
    let check = |id: &str| {
        result
            .checks
            .iter()
            .find(|c| c.acceptance_id == id)
            .unwrap_or_else(|| panic!("no result for {id}"))
    };
    assert_eq!(
        check("AC-X-001").exit_code,
        Some(0),
        "the authorized check must actually run"
    );
    assert!(check("AC-X-001").operational_error.is_none());
    let blocked = check("AC-X-002").operational_error.as_deref().unwrap_or("");
    assert!(
        blocked.contains("judge"),
        "the unauthorized check must fail as itself: {blocked}"
    );
    assert!(!result.passed());
    assert!(result.teardown_verified);
}

#[tokio::test]
async fn guardian_resolves_its_own_tools_through_the_configured_toolchain() {
    let (repo, head) = source_repository();
    let real_git = String::from_utf8(
        std::process::Command::new("/usr/bin/env")
            .args(["sh", "-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    assert!(!real_git.is_empty(), "the host must provide git");
    let toolchain = tempfile::tempdir().unwrap();
    let marker = toolchain.path().join("git-was-taken-from-the-toolchain");
    // Shadows git ahead of the host default directories. Only a child whose
    // PATH is the configured toolchain can reach it.
    std::fs::write(
        toolchain.path().join("git"),
        format!(
            "#!/bin/sh\n: >> '{}'\nexec '{real_git}' \"$@\"\n",
            marker.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            toolchain.path().join("git"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    let fixture = frozen_fixture(vec![criterion("AC-X-001", command("test -f input".into()))]);
    let scratch = tempfile::tempdir().unwrap();
    let evidence = scratch.path().join("evidence");
    let result = crate::command::acceptance_scratch_guardian::launch(request(
        &fixture,
        policy(
            repo.path(),
            fixture.project.path(),
            &fixture.task_root,
            scratch.path(),
            &format!("{}:/usr/bin:/bin", toolchain.path().display()),
            vec![],
        ),
        head,
        evidence.clone(),
    ))
    .await;
    if result.is_err() {
        let raw = std::fs::read_to_string(evidence.join("observation.json")).unwrap_or_default();
        panic!("DEBUG marker={} raw={}", marker.exists(), &raw[..raw.len().min(4000)]);
    }
    let result = result.unwrap();
    assert_eq!(result.checks[0].exit_code, Some(0));
    assert!(
        marker.exists(),
        "the guardian must resolve its tools through the policy toolchain, \
         not a hardcoded host PATH"
    );
}
