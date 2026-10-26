//! D1: a probe run that gave no verdict because of the check's own run goes
//! to the check's author at once; one that may be the host's follows the
//! #356 strike rule; and no such run is ever left unproven with no strike.

use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::TrustedCwd;

use super::probe_tests::{Trees, trees};
use super::*;
use crate::command::workflow_freeze_budget::{FreezeBudget, FreezeIncomplete, FreezeResume};

/// Prints past the hermetic site's output limit, then fails.
const LOUD: &str = "head -c 400000 /dev/zero | tr '\\0' x; exit 1";

/// Passes while its input is in place; signals its own wrapper, then fails,
/// once the input is moved aside.
const SIGNALLED: &str = "test -f data/state.txt || kill -TERM $$; test -f data/state.txt";

#[tokio::test]
async fn output_past_the_site_limit_goes_to_the_checks_author_at_once() {
    let trees = trees(&[("AC-X-001", LOUD, TrustedCwd::RepoRoot)]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = probe.take_unproven();
    let finding = findings
        .get("AC-X-001")
        .unwrap_or_else(|| panic!("no author finding: {findings:?}; unproven={unproven:?}"));
    assert!(finding.contains("output limit"), "{finding}");
    assert!(!unproven.contains_key("AC-X-001"), "{unproven:?}");
}

async fn staged(
    trees: &Trees,
) -> anyhow::Result<crate::command::workflow_task_set::coverage_gate::PreImplementation> {
    let resume = FreezeResume::saving(FreezeBudget::unlimited(), true);
    crate::command::workflow_task_set::coverage_gate::pre_implementation_findings(
        trees.set.project.path(),
        &trees.set.tasks,
        &trees.set.prd,
        &trees.contract(),
        &resume,
    )
    .await
}

fn author_finding(
    found: anyhow::Result<crate::command::workflow_task_set::coverage_gate::PreImplementation>,
    id: &str,
) -> String {
    let found = found.unwrap_or_else(|error| {
        let incomplete = FreezeIncomplete::caused(&error).map(FreezeIncomplete::report);
        panic!("no finding: the freeze ended {error:#} ({incomplete:?})")
    });
    let finding = (found.findings.iter())
        .find(|finding| finding.subject == id)
        .unwrap_or_else(|| panic!("no finding for {id}"));
    assert_eq!(
        finding.remediation_scope,
        archon_workflow::RemediationScope::CandidateArtifact,
        "{}",
        finding.text
    );
    finding.text.clone()
}

#[tokio::test]
async fn output_past_the_site_limit_is_an_author_finding_under_a_staged_freeze_never_a_pause() {
    let trees = trees(&[("AC-X-001", LOUD, TrustedCwd::RepoRoot)]);
    let finding = author_finding(staged(&trees).await, "AC-X-001");
    assert!(finding.contains("output limit"), "{finding}");
}

#[tokio::test]
async fn a_first_signal_stays_operational_and_the_second_is_an_author_finding() {
    let trees = trees(&[("AC-X-001", SIGNALLED, TrustedCwd::ProjectRoot)]);
    let first = staged(&trees)
        .await
        .expect("the probe returns its findings before the freeze decides to stall");
    let pending = first
        .findings
        .iter()
        .find(|finding| finding.subject == "AC-X-001")
        .expect("the host-unproven check remains visible to freeze orchestration");
    assert_eq!(
        pending.remediation_scope,
        archon_workflow::RemediationScope::Operational
    );
    assert!(pending.text.contains("signal TERM"), "{}", pending.text);
    assert!(first.host_unproven_reason.is_some());
    let finding = author_finding(staged(&trees).await, "AC-X-001");
    assert!(finding.contains("signal"), "{finding}");
}

/// One injected mutation run of `id`, as [`prove::prove`] reads it.
fn mutation_run(
    id: &str,
    markers: &mutation::Markers,
    signal: &str,
    error: Option<&str>,
) -> repairs::TreeRun {
    let result = CheckResult {
        fault: None,
        classification: None,
        acceptance_id: id.into(),
        exit_code: Some(1),
        quota_walk_count: 0,
        stdout: format!(
            "archon-mutation-state-{0}: 0:restored\narchon-mutation-status-{0}: {1}:1:none:{signal}\n",
            markers.nonce(),
            markers.tag(),
        )
        .into_bytes(),
        stderr: format!("{}0\n", markers.moved_prefix()).into_bytes(),
        operational_error: error.map(str::to_string),
        environment_note: None,
    };
    let evidence = hermetic::CheckEvidence {
        result,
        copy_layout: None,
    };
    repairs::TreeRun::known(&[evidence], "already observed")
}

async fn prove_once(
    trees: &Trees,
    markers: &mutation::Markers,
    run: repairs::TreeRun,
) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let contract = trees.contract();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let digest = contract_digest(&contract).unwrap();
    let refs = refs_for(&contract, &digest, &trees.ids());
    let baseline = Baseline {
        commit: trees.base.clone(),
        repository: trees.repo.clone(),
    };
    probe.inject_prove_run(run, markers.clone());
    let findings = prove::prove(
        &probe,
        &baseline,
        &contract,
        &digest,
        &refs,
        &["AC-X-001".into()],
        &BTreeMap::new(),
    )
    .await;
    (findings, probe.take_unproven())
}

#[tokio::test]
async fn a_mutation_run_past_the_output_limit_is_the_authors_at_once() {
    let trees = trees(&[(
        "AC-X-001",
        "test -f data/state.txt",
        TrustedCwd::ProjectRoot,
    )]);
    let markers = mutation::Markers::new();
    let run = mutation_run(
        "AC-X-001",
        &markers,
        "none",
        Some("native acceptance output limit exceeded"),
    );
    let (findings, unproven) = prove_once(&trees, &markers, run).await;
    let finding = findings
        .get("AC-X-001")
        .unwrap_or_else(|| panic!("no author finding: {findings:?}; unproven={unproven:?}"));
    assert!(finding.contains("output limit"), "{finding}");
}

#[tokio::test]
async fn a_signal_in_a_mutation_run_is_struck_then_goes_to_its_author_the_second_time() {
    let trees = trees(&[(
        "AC-X-001",
        "test -f data/state.txt",
        TrustedCwd::ProjectRoot,
    )]);
    let markers = mutation::Markers::new();
    let (findings, unproven) = prove_once(
        &trees,
        &markers,
        mutation_run("AC-X-001", &markers, "TERM", None),
    )
    .await;
    assert!(
        findings.is_empty(),
        "the first signal may be the host's: {findings:?}"
    );
    assert!(unproven.contains_key("AC-X-001"), "{unproven:?}");
    let (findings, unproven) = prove_once(
        &trees,
        &markers,
        mutation_run("AC-X-001", &markers, "TERM", None),
    )
    .await;
    let finding = findings.get("AC-X-001").unwrap_or_else(|| {
        panic!("the same signal again is the author's: {findings:?}; unproven={unproven:?}")
    });
    assert!(finding.contains("signal TERM"), "{finding}");
}

/// A check carrying a host policy defect is never run, and never run beside
/// the others: one such check used to leave every check of its run without
/// a verdict, the host's, so a sibling that would have proved itself could
/// not be published.
#[tokio::test]
async fn a_policy_defect_of_one_check_never_leaves_its_sibling_unproven() {
    let trees = trees(&[
        (
            "AC-X-001",
            "printf rebuilt > data/state.txt; exit 0",
            TrustedCwd::ProjectRoot,
        ),
        (
            "AC-X-002",
            "test -f data/state.txt",
            TrustedCwd::ProjectRoot,
        ),
    ]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let _ = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = probe.take_unproven();
    assert!(
        unproven["AC-X-001"].contains("host policy defect"),
        "{unproven:?}"
    );
    assert!(!unproven.contains_key("AC-X-002"), "{unproven:?}");
}

/// H5: a fault the observation recorded with its typed cause is judged by
/// that cause, never by its message text.
#[test]
fn a_typed_fault_cause_is_judged_by_the_cause_not_the_text() {
    use archon_workflow::acceptance_scratch::faults::FaultCause;
    let markers = mutation::Markers::new();
    let run = mutation_run(
        "AC-X-001",
        &markers,
        "none",
        Some("the site could not be read"),
    );
    let mut result = run.results["AC-X-001"].result.clone();
    assert!(matches!(
        check_fault::of_result(Some(&result)),
        check_fault::Fault::Host
    ));
    result.fault = Some(FaultCause::Removed {
        part: "source".into(),
    });
    assert!(matches!(
        check_fault::of_result(Some(&result)),
        check_fault::Fault::Check { .. }
    ));
}
