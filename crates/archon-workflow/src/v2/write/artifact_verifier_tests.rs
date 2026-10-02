//! Batch G2 (G2-5, G2-3b): the artifact verifier is supervised like every
//! host-run command, and what the environment did is never the branch's.
use super::*;
use crate::write_coordinator::project_inputs::write_test_policy;

#[cfg(unix)]
fn child_alive(pidfile: &Path) -> bool {
    crate::v2::write::test_baseline_run::child_alive_for_tests(pidfile)
}

fn accepted() -> WorkflowV2Result {
    WorkflowV2Result::accepted("artifact written")
}

fn input(commands: &[&str]) -> serde_json::Value {
    serde_json::json!({"item": {"artifact_verification_commands": commands}})
}

/// A verifier past its wall clock is ended with its whole process group --
/// a child it started in the background never gets to run on -- and that
/// is no verdict, not the branch's failure.
#[cfg(unix)] // Requires Unix process-group teardown, not just leader termination.
#[test]
fn a_hung_verifier_is_cut_and_its_group_killed() {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("child.pid");
    let command = format!("sleep 60 & echo $! > {}; sleep 30", pidfile.display());
    let started = Instant::now();
    let outcome = run_supervised(&command, dir.path(), Duration::from_millis(500), None, None);
    assert!(started.elapsed() < Duration::from_secs(10), "{outcome:?}");
    match outcome {
        Err(VerifierFailure::Environment(reason)) => {
            assert!(reason.contains("did not finish within"), "{reason}")
        }
        other => panic!("expected no verdict, got {other:?}"),
    }
    assert!(
        !child_alive(&pidfile),
        "the background child was not killed"
    );
}

#[test]
fn an_exit_is_the_branchs_failure_and_a_spawn_failure_is_not() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        run_supervised("true", dir.path(), Duration::from_secs(30), None, None),
        Ok(())
    );
    match run_supervised(
        "echo missing >&2; exit 3",
        dir.path(),
        Duration::from_secs(30),
        None,
        None,
    ) {
        Err(VerifierFailure::Product(text)) => assert!(text.contains("missing"), "{text}"),
        other => panic!("expected the branch's failure, got {other:?}"),
    }
    let gone = dir.path().join("no-such-worktree");
    assert!(matches!(
        run_supervised("true", &gone, Duration::from_secs(30), None, None),
        Err(VerifierFailure::Environment(_))
    ));
}

/// No verdict twice is the host's operational error, which every classifier
/// files with transport failures; a product failure stays the branch's.
#[test]
fn no_verdict_twice_is_the_hosts_operational_error() {
    let dir = tempfile::tempdir().unwrap();
    let hung = input(&["sleep 30"]);
    let error = verify_with_timeout(
        &hung,
        &accepted(),
        dir.path(),
        None,
        Duration::from_millis(200),
    )
    .expect_err("no verdict");
    assert!(crate::error::is_host_operational_text(&error), "{error}");
    assert_eq!(
        super::super::super::errors::write_branch_error_kind(&error),
        crate::v2::BranchFailureKind::Execution
    );
    let result =
        super::super::super::errors::write_branch_validation_error_result("a", None, &error);
    assert_eq!(
        result.data["transport_failure_no_verdict"], true,
        "{result:#?}"
    );

    let failed = input(&["exit 1"]);
    let error = verify_with_timeout(
        &failed,
        &accepted(),
        dir.path(),
        None,
        Duration::from_secs(30),
    )
    .expect_err("the branch's failure");
    assert!(!crate::error::is_host_operational_text(&error), "{error}");
}

/// G2-3b: a verifier during whose run the project's inputs changed gave no
/// trusted verdict. The host puts the input back and re-runs it alone; the
/// re-run's clean pass stands, and the branch is not charged.
#[test]
fn a_verifier_run_that_changed_an_input_is_restored_and_re_run() {
    // What the host does once a change got through (off macOS, or from an
    // unbounded process): the boundary itself is tested below.
    crate::write_coordinator::host_sandbox::unbounded_for_tests(true);
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let run_root = project.join(".archon/workflows/run1");
    let data = project.join(".archon/lab/data/registry.json");
    std::fs::create_dir_all(&run_root).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    std::fs::create_dir_all(data.parent().unwrap()).unwrap();
    std::fs::write(&data, "original").unwrap();
    write_test_policy(&run_root, &project, &[".archon/lab"]);
    let worktree = dir.path().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    // First run only: rewrites the live input (as a sibling's command could).
    let command = format!(
        "if [ ! -f once ]; then touch once; printf changed > {}; fi",
        data.display()
    );
    let outcome = verify_with_timeout(
        &input(&[&command]),
        &accepted(),
        &worktree,
        Some(&run_root),
        Duration::from_secs(30),
    );
    assert_eq!(outcome, Ok(()));
    assert_eq!(std::fs::read_to_string(&data).unwrap(), "original");

    // Every run changes it: the host's operational error, not the branch's.
    let always = format!("printf changed > {}", data.display());
    let error = verify_with_timeout(
        &input(&[&always]),
        &accepted(),
        &worktree,
        Some(&run_root),
        Duration::from_secs(30),
    )
    .expect_err("no trusted verdict");
    assert!(crate::error::is_host_operational_text(&error), "{error}");
    assert!(error.contains("ENVIRONMENT VIOLATION"), "{error}");
    assert_eq!(std::fs::read_to_string(&data).unwrap(), "original");
}

/// G2 (Invariant 1): under the host boundary a verifier cannot write the
/// run store or the project root, and its write fails as the branch's own
/// verifier failing, the files untouched; its own worktree stays writable.
#[test]
fn a_verifier_cannot_write_the_hosts_roots() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().canonicalize().unwrap().join("project");
    let run_root = project.join(".archon/workflows/run1");
    let worktree = run_root.join("v2/worktrees/impl/impl-0");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    write_test_policy(&run_root, &project, &[".archon/lab"]);
    let record = run_root.join("state.json");
    std::fs::write(&record, "host").unwrap();
    let outcome = run_supervised(
        &format!("printf forged > {}", record.display()),
        &worktree,
        Duration::from_secs(30),
        Some(&run_root),
        None,
    );
    if cfg!(target_os = "macos") {
        assert!(
            matches!(outcome, Err(VerifierFailure::Product(_))),
            "{outcome:?}"
        );
    }
    assert_eq!(std::fs::read_to_string(&record).unwrap(), "host");
    let own = run_supervised(
        "printf ok > own.txt",
        &worktree,
        Duration::from_secs(30),
        Some(&run_root),
        None,
    );
    assert_eq!(own, Ok(()));
}
