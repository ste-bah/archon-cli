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

/// 34 realistic findings (40-80 chars each), as a floor or a verifier
/// reports them.
fn many_findings() -> Vec<String> {
    (1..=34)
        .map(|n| format!("records[{n}].close is missing or not a finite number (row {n})"))
        .collect()
}

/// Issue 219: every finding a failed contract reports reaches the residual
/// gap remediation reads, and the branch data keeps them whole.
fn assert_every_finding_reaches_the_gap(findings: &[String]) {
    let mut outcome = accepted("v");
    demote_failed_contract(&mut outcome, findings, None);
    let result = outcome.result.expect("result");
    let gap = &result.residual_gaps[0].description;
    assert!(
        gap.contains(&format!("{} finding(s)", findings.len())),
        "{gap}"
    );
    for finding in findings {
        assert!(
            gap.contains(finding.as_str()),
            "missing `{finding}` in {gap}"
        );
    }
    assert_eq!(
        result.data["declared_contract_findings"],
        serde_json::json!(findings)
    );
}

#[test]
fn every_declarative_floor_finding_reaches_the_residual_gap() {
    let findings = many_findings();
    assert!(findings.iter().all(|f| (40..=80).contains(&f.len())));
    let ContractVerification::Failed(reported, _) = floor_failed(&findings) else {
        panic!("expected a failure");
    };
    assert_every_finding_reaches_the_gap(&reported);
}

#[test]
fn every_verifier_failure_reaches_the_residual_gap() {
    let findings = many_findings();
    let verdict = serde_json::json!({"status": "failed", "failures": findings});
    let reported = verdict_failure(&verdict).expect("a failure");
    assert_eq!(reported, findings);
    assert_every_finding_reaches_the_gap(&reported);
}

/// A finding longer than the gap quotes is cut with a mark naming its
/// length; the next finding still follows it.
#[test]
fn an_oversized_finding_is_cut_with_a_mark_and_drops_no_neighbour() {
    let findings = vec!["y".repeat(9000), "records[2].open is missing".to_string()];
    let mut outcome = accepted("v");
    demote_failed_contract(&mut outcome, &findings, None);
    let gap = &outcome.result.expect("result").residual_gaps[0].description;
    assert!(gap.contains("[finding cut at 4096 of 9000 bytes"), "{gap}");
    assert!(gap.contains("records[2].open is missing"), "{gap}");
}

/// Issue 219 round 3: a thousand findings (one per missing instance) keep
/// the gap -- what every remediate and verify prompt quotes -- and the data
/// within budget; the evidence file in the run directory holds all of them,
/// and the gap names it.
#[test]
fn a_thousand_findings_stay_within_budget_and_all_reach_the_evidence_file() {
    let run = tempfile::tempdir().unwrap();
    let findings: Vec<String> = (1..=1000)
        .map(|n| format!("records[{n}] declared instance is missing from the artifact"))
        .collect();
    let mut outcome = accepted("TASK-1");
    demote_failed_contract(&mut outcome, &findings, Some(run.path()));
    let result = outcome.result.expect("result");
    let gap = &result.residual_gaps[0].description;
    assert!(
        gap.len() <= demote::GAP_BUDGET_BYTES + 1024,
        "{}",
        gap.len()
    );
    let path = result.data["declared_contract_findings_path"]
        .as_str()
        .expect("the evidence path");
    assert!(
        path.starts_with(&run.path().display().to_string()),
        "{path}"
    );
    assert!(
        gap.contains(&format!("more finding(s): full list at {path}")),
        "{gap}"
    );
    assert!(gap.contains("1000 finding(s)"), "{gap}");
    let written: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(written["findings"], serde_json::json!(findings));
    let shown = result.data["declared_contract_findings"]
        .as_array()
        .unwrap();
    assert!(!shown.is_empty() && shown.len() < 1000, "{}", shown.len());
    assert_eq!(result.data["declared_contract_finding_count"], 1000);
    assert!(serde_json::to_string(shown).unwrap().len() <= demote::GAP_BUDGET_BYTES + 1024);
}

/// Issue 219 round 3: a verifier whose stdout passes the read cap is cut
/// with its byte count, and a pass printed before the cut never stands.
#[cfg(unix)]
#[tokio::test]
async fn verifier_output_past_the_cap_is_cut_with_its_byte_count_and_never_passes() {
    let command = r#"printf '{"status":"verified"}\n'; head -c 2000000 /dev/zero | tr '\0' 'x'"#;
    match run_contract_verifier_within(command, std::time::Duration::from_secs(60)).await {
        ContractVerification::Failed(findings, _) => {
            let text = findings.join("; ");
            assert!(text.contains("[output cut]"), "{text}");
            assert!(text.contains("of 2000022 bytes"), "{text}");
        }
        _ => panic!("a cut verdict must never pass"),
    }
}

#[cfg(unix)]
fn contract_note_case(case: &str, json: &str, exit: i32) {
    if std::env::var("ISSUE_349_CASE").as_deref() != Ok(case) {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([case, "--nocapture"])
            .env("ISSUE_349_CASE", case)
            .env("FIXTURE_API_KEY", "hidden-data")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let script = format!("printf '%s\\n' '{}'; exit {exit}", json);
    match runtime.block_on(run_contract_verifier(&script)) {
        ContractVerification::Failed(findings, Some(note)) => {
            assert!(
                note.starts_with("Note:")
                    && note.contains("FIXTURE_API_KEY")
                    && !note.contains("hidden-data")
            );
            assert_every_finding_reaches_the_gap(&findings);
            let mut outcome = accepted("verifier");
            demote_failed_contract(&mut outcome, &findings, None);
            demote::attach_environment_note(&mut outcome, &note);
            assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
            assert_eq!(outcome.failure_kind, Some(BranchFailureKind::Semantic));
            let result = outcome.result.unwrap();
            assert_eq!(
                result.data["declared_contract_finding_count"],
                findings.len()
            );
            assert_eq!(result.data["check_environment_note"], note);
            assert!(result.residual_gaps[0].description.contains(&note));
        }
        _ => panic!("output changed the real failure verdict"),
    }
    let script = r#"printf '%s\n' '{"status":"verified","message":"FIXTURE_API_KEY is not set"}'"#;
    assert!(matches!(
        runtime.block_on(run_contract_verifier(script)),
        ContractVerification::Passed
    ));
}

#[cfg(unix)]
#[test]
fn r4_contract_json_failure_exit_one() {
    contract_note_case(
        "r4_contract_json_failure_exit_one",
        r#"{"status":"failed","failures":["FIXTURE_API_KEY environment variable is not set"]}"#,
        1,
    );
}
#[cfg(unix)]
#[test]
fn r4_contract_json_failure_exit_zero() {
    contract_note_case(
        "r4_contract_json_failure_exit_zero",
        r#"{"status":"failed","failures":["FIXTURE_API_KEY environment variable is not set"]}"#,
        0,
    );
}
#[cfg(unix)]
#[test]
fn r4_contract_json_quoted_expectation() {
    contract_note_case(
        "r4_contract_json_quoted_expectation",
        r#"{"status":"failed","failures":["expected \"FIXTURE_API_KEY environment variable is not set\" in stderr"]}"#,
        0,
    );
}
