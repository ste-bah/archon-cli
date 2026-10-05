//! Batch G2 (G2-3a, G2-3c, G2-5): a declared contract verifier that gives no
//! verdict, or during whose run the project's inputs changed, is the host's
//! to resolve; it never demotes the branch as a contract violation and never
//! touches a sibling's verdict.
use super::*;
use crate::v2::WorkflowV2Result;
use crate::write_coordinator::project_inputs::write_test_policy;

fn accepted(item_id: &str) -> WorkflowV2BranchOutcome {
    WorkflowV2BranchOutcome {
        item_id: item_id.to_string(),
        role: "verifier".to_string(),
        status: WorkflowV2Status::Accepted,
        result: Some(WorkflowV2Result::accepted("verified")),
        error: None,
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: Vec::new(),
    }
}

/// A verifier past its wall clock gives no verdict, and its whole process
/// group is killed (a background child never runs on).
#[cfg(unix)] // Requires Unix process-group teardown, not just leader termination.
#[tokio::test]
async fn a_hung_contract_verifier_gives_no_verdict_and_its_group_is_killed() {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("child.pid");
    let command = format!("sleep 60 & echo $! > {}; sleep 30", pidfile.display());
    let started = std::time::Instant::now();
    let verification =
        run_contract_verifier_within(&command, std::time::Duration::from_millis(300)).await;
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    match verification {
        ContractVerification::Unavailable(reason) => {
            assert!(reason.contains("did not finish"), "{reason}")
        }
        _ => panic!("expected no verdict"),
    }
    assert!(
        !crate::v2::write::test_baseline_run::child_alive_for_tests(&pidfile),
        "the verifier's background child was not killed"
    );
}

struct Fixture {
    _dir: tempfile::TempDir,
    run_root: std::path::PathBuf,
    root: String,
    data: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let project = dir
        .path()
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap()
        .join("project");
    let run_root = project.join(".archon/workflows/run1");
    let data = project.join(".archon/lab/data/registry.json");
    std::fs::create_dir_all(&run_root).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    std::fs::create_dir_all(data.parent().unwrap()).unwrap();
    std::fs::write(&data, "original").unwrap();
    std::fs::write(project.join(".archon/lab/present.json"), r#"{"ok":true}"#).unwrap();
    write_test_policy(&run_root, &project, &[".archon/lab"]);
    Fixture {
        _dir: dir,
        run_root,
        root: project.display().to_string(),
        data,
    }
}

fn contract(typed: &str) -> serde_json::Value {
    serde_json::json!({"artifact_path": ".archon/lab/present.json", "typed_verifier_command": typed})
}

/// G2-3c: one branch's verifier changed the project's inputs on every run.
/// That branch alone fails as the host's operational error (typed
/// `Execution`, no result, the host's marker); its sibling's verdict stands
/// -- the old rule failed EVERY branch of the wave -- and the input is back.
#[tokio::test]
async fn only_the_branch_whose_verifier_changed_an_input_is_failed_and_as_operational() {
    // What the host does once a change got through (off macOS, or from an
    // unbounded process); the boundary is `host_sandbox`'s own test.
    crate::write_coordinator::host_sandbox::unbounded_for_tests(true);
    let f = fixture();
    let writes = format!(
        "printf changed > {}; printf '%s\\n' '{{\"status\":\"verified\"}}'",
        crate::acceptance_scratch::shell_arg(&f.data)
    );
    let clean = "printf '%s\\n' '{\"status\":\"verified\"}'";
    let mut outcomes = [accepted("writer"), accepted("innocent")];
    let contracts = std::collections::BTreeMap::from([
        (
            "writer".to_string(),
            (
                ContractRoots::project_only(f.root.clone()),
                vec![contract(&writes)],
            ),
        ),
        (
            "innocent".to_string(),
            (
                ContractRoots::project_only(f.root.clone()),
                vec![contract(clean)],
            ),
        ),
    ]);
    enforce_declared_contracts_watched(&mut outcomes, &contracts, Some(&f.run_root)).await;
    assert_eq!(std::fs::read_to_string(&f.data).unwrap(), "original");
    let writer = &outcomes[0];
    assert_eq!(writer.status, WorkflowV2Status::Failed);
    assert_eq!(writer.failure_kind, Some(BranchFailureKind::Execution));
    assert!(writer.result.is_none());
    let error = writer.error.as_deref().unwrap_or_default();
    assert!(crate::error::is_host_operational_text(error), "{error}");
    let innocent = &outcomes[1];
    assert_eq!(innocent.status, WorkflowV2Status::Accepted, "{innocent:#?}");
    assert_eq!(
        innocent.result.as_ref().unwrap().data["declared_contract_verification"],
        "passed"
    );
}

/// A change on the first run only: the host restores it, re-runs the
/// branch's verifiers alone, and the clean verdict stands.
#[tokio::test]
async fn a_verifier_that_changed_an_input_once_is_re_run_and_its_verdict_stands() {
    crate::write_coordinator::host_sandbox::unbounded_for_tests(true);
    let f = fixture();
    let flag = f.run_root.join("once");
    let once = format!(
        "if [ ! -f {flag} ]; then touch {flag}; printf changed > {data}; fi; printf '%s\\n' '{{\"status\":\"verified\"}}'",
        flag = crate::acceptance_scratch::shell_arg(&flag),
        data = crate::acceptance_scratch::shell_arg(&f.data)
    );
    let mut outcomes = [accepted("once")];
    let contracts = std::collections::BTreeMap::from([(
        "once".to_string(),
        (
            ContractRoots::project_only(f.root.clone()),
            vec![contract(&once)],
        ),
    )]);
    enforce_declared_contracts_watched(&mut outcomes, &contracts, Some(&f.run_root)).await;
    assert_eq!(std::fs::read_to_string(&f.data).unwrap(), "original");
    assert_eq!(
        outcomes[0].status,
        WorkflowV2Status::Accepted,
        "{:#?}",
        outcomes[0]
    );
}

/// G2 (Invariant 1): under the host boundary a contract verifier that tries
/// to write the project's inputs is refused; the input is untouched and the
/// branch is judged by the verdict it printed.
#[tokio::test]
async fn a_contract_verifier_cannot_write_the_project() {
    let f = fixture();
    let writes = format!(
        "printf changed > {}; printf '%s\\n' '{{\"status\":\"verified\"}}'",
        f.data.display()
    );
    let mut outcomes = [accepted("writer")];
    let contracts = std::collections::BTreeMap::from([(
        "writer".to_string(),
        (
            ContractRoots::project_only(f.root.clone()),
            vec![contract(&writes)],
        ),
    )]);
    enforce_declared_contracts_watched(&mut outcomes, &contracts, Some(&f.run_root)).await;
    assert_eq!(std::fs::read_to_string(&f.data).unwrap(), "original");
    // Issue-227: bounded on macOS and Linux; elsewhere the verifier is
    // refused (no verdict) rather than run unbounded.
    if crate::write_coordinator::host_sandbox::available() {
        assert_eq!(
            outcomes[0].status,
            WorkflowV2Status::Accepted,
            "{:#?}",
            outcomes[0]
        );
    }
}

/// Issue 219: a declarative floor with seven findings fails the branch
/// naming all seven, never the first five.
#[test]
fn every_declarative_floor_finding_reaches_the_failure() {
    let findings: Vec<String> = (1..=7).map(|n| format!("floor finding {n}")).collect();
    match floor_failed(&findings) {
        ContractVerification::Failed(detail) => {
            for finding in &findings {
                assert!(detail.contains(finding.as_str()), "{detail}");
            }
        }
        _ => panic!("expected a failure"),
    }
}
